use super::*;

fn bits(w: u32, hi: u32, lo: u32) -> u32 {
    (w >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

fn sext(v: u32, n: u32) -> i64 {
    let s = 64 - n;
    ((v as u64) << s) as i64 >> s
}

fn rsp(r: u32) -> u8 {
    if r == 31 {
        SP
    } else {
        r as u8
    }
}

pub fn vldst(w: u32) -> Insn {
    let size = bits(w, 31, 30);
    let opc = bits(w, 23, 22);
    let rt = bits(w, 4, 0) as u8;
    let rn = rsp(bits(w, 9, 5));
    if w & 0x3b000000 == 0x18000000 {
        let off = sext(bits(w, 23, 5), 19) << 2;
        return match size {
            0 => Insn::VLd { size: 2, rt, rn: 0, addr: Addr::Lit { off } },
            1 => Insn::VLd { size: 3, rt, rn: 0, addr: Addr::Lit { off } },
            2 => Insn::VLd { size: 4, rt, rn: 0, addr: Addr::Lit { off } },
            _ => Insn::Unsup("vldst-bad"),
        };
    }
    if w & 0x3a000000 == 0x28000000 {
        let ty = bits(w, 24, 23);
        let l = bits(w, 22, 22) != 0;
        let sz = match size {
            0 => 2u8,
            1 => 3,
            2 => 4,
            _ => return Insn::Unsup("vldst-bad"),
        };
        let off = sext(bits(w, 21, 15), 7) << sz;
        let rt2 = bits(w, 14, 10) as u8;
        let mode = match ty {
            0 | 2 => Mode::Offset,
            1 => Mode::Post,
            _ => Mode::Pre,
        };
        return if l {
            Insn::VLdp { sz, rt, rt2, rn, off, mode }
        } else {
            Insn::VStp { sz, rt, rt2, rn, off, mode }
        };
    }
    if w & 0x3a000000 == 0x38000000 {
        let lsz = if opc & 2 != 0 {
            if size != 0 {
                return Insn::Unsup("vldst-bad");
            }
            4u8
        } else {
            size as u8
        };
        let load = opc & 1 != 0;
        let mk = |addr: Addr| {
            if load {
                Insn::VLd { size: lsz, rt, rn, addr }
            } else {
                Insn::VSt { size: lsz, rt, rn, addr }
            }
        };
        if bits(w, 25, 24) == 1 {
            let off = (bits(w, 21, 10) as i64) << lsz;
            return mk(Addr::Imm { off, mode: Mode::Offset });
        }
        if bits(w, 21, 21) == 0 {
            let imm9 = sext(bits(w, 20, 12), 9);
            let mode = match bits(w, 11, 10) {
                0 | 2 => Mode::Offset,
                1 => Mode::Post,
                _ => Mode::Pre,
            };
            return mk(Addr::Imm { off: imm9, mode });
        }
        if bits(w, 11, 10) == 2 {
            let ext = match bits(w, 15, 13) {
                2 => Ext::Uxtw,
                3 => Ext::Uxtx,
                6 => Ext::Sxtw,
                7 => Ext::Sxtx,
                _ => return Insn::Unsup("vldst-bad"),
            };
            let amt = if bits(w, 12, 12) != 0 { lsz } else { 0 };
            return mk(Addr::Reg { rm: bits(w, 20, 16) as u8, ext, amt });
        }
    }
    Insn::Unsup("simd-ldst-other")
}

fn vfp_expand(imm8: u32, ftype: u32) -> u64 {
    let a = (imm8 >> 7) & 1;
    let b = (imm8 >> 6) & 1;
    let cdefgh = imm8 & 0x3f;
    if ftype == 0 {
        let e = if b == 1 { 0x1f } else { 0x20 };
        ((a << 31) | (e << 25) | (cdefgh << 19)) as u64
    } else {
        let e: u64 = if b == 1 { 0x0ff } else { 0x100 };
        ((a as u64) << 63) | (e << 54) | ((cdefgh as u64) << 48)
    }
}

fn movi_imm(op: u32, cmode: u32, imm8: u32) -> Option<(u64, bool)> {
    let i = imm8 as u64;
    let rep32 = |v: u64| v | (v << 32);
    let (v, inv) = match cmode {
        0 | 2 | 4 | 6 => (rep32(i << (8 * (cmode >> 1))), op == 1),
        8 | 10 => {
            let v = i << (8 * ((cmode >> 1) & 1));
            (v | (v << 16) | (v << 32) | (v << 48), op == 1)
        }
        12 | 13 => {
            let v = if cmode == 12 { (i << 8) | 0xff } else { (i << 16) | 0xffff };
            (rep32(v), op == 1)
        }
        14 => {
            if op == 0 {
                (i * 0x0101_0101_0101_0101, false)
            } else {
                let mut v = 0u64;
                for k in 0..8 {
                    if (imm8 >> k) & 1 != 0 {
                        v |= 0xffu64 << (8 * k);
                    }
                }
                (v, false)
            }
        }
        _ => return None,
    };
    if matches!(cmode, 1 | 3 | 5 | 7 | 9 | 11) {
        return None;
    }
    Some((v, inv))
}

pub fn simd(w: u32) -> Insn {
    let rd = bits(w, 4, 0) as u8;
    let rn = bits(w, 9, 5) as u8;
    let rm = bits(w, 20, 16) as u8;
    if w & 0x7f200000 == 0x1e200000 {
        let ftype = bits(w, 23, 22);
        let sf = w >> 31 != 0;
        if bits(w, 15, 10) == 0 {
            let rmode = bits(w, 20, 19);
            let opc = bits(w, 18, 16);
            if ftype == 2 && !(rmode == 1 && (opc == 6 || opc == 7)) || ftype == 3 {
                return Insn::Unsup("fp-conv-h");
            }
            return match (rmode, opc) {
                (0, 2) => Insn::CvtIF { sf, ftype: ftype as u8, rd, rn, signed: true },
                (0, 3) => Insn::CvtIF { sf, ftype: ftype as u8, rd, rn, signed: false },
                (3, 0) => Insn::CvtFI { sf, ftype: ftype as u8, rd, rn, signed: true },
                (3, 1) => Insn::CvtFI { sf, ftype: ftype as u8, rd, rn, signed: false },
                // FMOV general<->FP only pairs W with S and X with D.
                (0, 6) if sf == (ftype == 1) => Insn::FmovToG { sf, ftype: ftype as u8, top: false, rd, rn },
                (0, 7) if sf == (ftype == 1) => Insn::FmovFromG { sf, ftype: ftype as u8, top: false, rd, rn },
                (1, 6) if ftype == 2 && sf => Insn::FmovToG { sf, ftype: 1, top: true, rd, rn },
                (1, 7) if ftype == 2 && sf => Insn::FmovFromG { sf, ftype: 1, top: true, rd, rn },
                _ => Insn::Unsup("fp-conv"),
            };
        }
        // Outside the int<->fp conversions, bit 31 is M and must be 0.
        if sf {
            return Insn::Unsup("fp-m");
        }
        if ftype >= 2 {
            return Insn::Unsup("fp-half");
        }
        let ft = ftype as u8;
        if bits(w, 14, 10) == 0b10000 {
            let opcode = bits(w, 20, 15);
            return match opcode {
                0 => Insn::FmovR { ftype: ft, rd, rn },
                1 => Insn::Fp1 { ftype: ft, op: 1, rd, rn },
                2 => Insn::Fp1 { ftype: ft, op: 2, rd, rn },
                3 => Insn::Fp1 { ftype: ft, op: 3, rd, rn },
                // Opcode names the destination: 4 = to single, 5 = to double.
                5 if ftype == 0 => Insn::Fcvt { from: 0, to: 1, rd, rn },
                4 if ftype == 1 => Insn::Fcvt { from: 1, to: 0, rd, rn },
                _ => Insn::Unsup("fp-1src"),
            };
        }
        if bits(w, 13, 10) == 0b1000 && bits(w, 15, 14) == 0 {
            let opc2 = bits(w, 4, 0);
            return match opc2 {
                0 | 8 | 16 | 24 => Insn::Fcmp { ftype: ft, rn, rm, zero: opc2 & 8 != 0, e: opc2 & 16 != 0 },
                _ => Insn::Unsup("fp-cmp"),
            };
        }
        if bits(w, 12, 10) == 0b100 {
            if bits(w, 9, 5) != 0 {
                return Insn::Unsup("fp-imm");
            }
            return Insn::FmovI { ftype: ft, rd, imm: vfp_expand(bits(w, 20, 13), ftype) };
        }
        return match bits(w, 11, 10) {
            1 => Insn::Unsup("fp-ccmp"),
            2 => {
                let op = bits(w, 15, 12);
                match op {
                    0 | 1 | 2 | 3 => Insn::Fp2 { ftype: ft, op: op as u8, rd, rn, rm },
                    _ => Insn::Unsup("fp-2src"),
                }
            }
            3 => Insn::Fcsel { ftype: ft, rd, rn, rm, cond: bits(w, 15, 12) as u8 },
            _ => Insn::Unsup("fp-other"),
        };
    }
    if w & 0x7f000000 == 0x1f000000 {
        let ftype = bits(w, 23, 22);
        if w >> 31 != 0 {
            return Insn::Unsup("fp-m");
        }
        if ftype >= 2 {
            return Insn::Unsup("fp-half");
        }
        let o = (bits(w, 21, 21) << 1) | bits(w, 15, 15);
        return Insn::Fp3 { ftype: ftype as u8, op: o as u8, rd, rn, rm, ra: bits(w, 14, 10) as u8 };
    }
    if w & 0x9f200400 == 0x0e200400 {
        let q = bits(w, 30, 30) != 0;
        let u = bits(w, 29, 29);
        let size = bits(w, 23, 22);
        let opcode = bits(w, 15, 11);
        return match (opcode, u) {
            (3, _) => {
                let op = (u << 2) | size;
                Insn::VLog { op: op as u8, q, rd, rn, rm }
            }
            (16, _) => {
                if q || size != 3 {
                    Insn::VArith { sub: u == 1, size: size as u8, q, rd, rn, rm }
                } else {
                    Insn::Unsup("vec-3same")
                }
            }
            _ => Insn::Unsup("vec-3same"),
        };
    }
    if w & 0x9fe08400 == 0x0e000400 {
        let q = bits(w, 30, 30) != 0;
        let op = bits(w, 29, 29);
        let imm5 = bits(w, 20, 16);
        let imm4 = bits(w, 14, 11);
        if imm5 & 0xf == 0 {
            return Insn::Unsup("vec-copy");
        }
        let size = imm5.trailing_zeros() as u8;
        let idx = (imm5 >> (size + 1)) as u8;
        return match (op, imm4) {
            // 64-bit elements need Q=1 (.1D is reserved); INS (general) is Q=1 only.
            (0, 1) if size < 3 || q => Insn::VDupG { q, size, rd, rn },
            (0, 7) if (size == 3 && q) || (size < 3 && !q) => Insn::Umov { sf: size == 3, size, idx, rd, rn },
            (0, 3) if q => Insn::InsG { size, idx, rd, rn },
            (0, 0) if size < 3 || q => Insn::VDupE { q, size, idx, rd, rn },
            _ => Insn::Unsup("vec-copy"),
        };
    }
    if w & 0x9ff80400 == 0x0f000400 {
        let q = bits(w, 30, 30) != 0;
        let op = bits(w, 29, 29);
        let cmode = bits(w, 15, 12);
        let imm8 = (bits(w, 18, 16) << 5) | bits(w, 9, 5);
        return match movi_imm(op, cmode, imm8) {
            Some((v, inv)) => {
                let v = if inv { !v } else { v };
                Insn::Movi { q, rd, imm: v }
            }
            None => Insn::Unsup("vec-modimm"),
        };
    }
    if w & 0x9f3e0c00 == 0x0e200800 {
        return Insn::Unsup("vec-2reg");
    }
    if w & 0xdf200400 == 0x5e200400 {
        return Insn::Unsup("scalar-3same");
    }
    if w & 0xdf3e0c00 == 0x5e200800 {
        return Insn::Unsup("scalar-2reg");
    }
    if w & 0x9f000400 == 0x0f000400 {
        return Insn::Unsup("vec-shift-imm");
    }
    if w & 0x9f000400 == 0x0f000000 {
        return Insn::Unsup("vec-elem");
    }
    if w & 0xbf208c00 == 0x0e000800 {
        return Insn::Unsup("vec-perm");
    }
    if w & 0xbf20fc00 == 0x0e00_0000 {
        return Insn::Unsup("vec-tbl");
    }
    if w & 0xbfe08c00 == 0x2e000000 {
        return Insn::Unsup("vec-ext");
    }
    if w & 0xbf3e0c00 == 0x0e300800 {
        return Insn::Unsup("vec-across");
    }
    if w & 0xff000000 == 0x4e000000 && bits(w, 28, 24) == 0xe {
        return Insn::Unsup("crypto");
    }
    Insn::Unsup("simd-other")
}
