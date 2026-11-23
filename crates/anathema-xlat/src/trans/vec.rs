use super::*;

const X13: u8 = 13;
const X14: u8 = 14;
const X15: u8 = 15;

fn pp_of(ft: u8) -> u8 {
    if ft == 0 {
        2
    } else {
        3
    }
}

impl<'a> Tx<'a> {
    fn vpin(&self, v: u8) -> Option<u8> {
        let p = self.cfg.vpins[v as usize];
        if p == NOPIN {
            None
        } else {
            Some(p)
        }
    }

    fn vslot(v: u8) -> Mem {
        Mem::b(CTX, OFF_Q + v as i32 * 16 - 128)
    }

    fn vrm(&self, v: u8) -> Rm {
        match self.vpin(v) {
            Some(p) => Rm::R(p),
            None => Rm::M(Self::vslot(v)),
        }
    }

    fn vsrc(&mut self, v: u8, tmp: u8) -> u8 {
        match self.vpin(v) {
            Some(p) => p,
            None => {
                self.a.vex(2, 1, false, 0, tmp, Rm::M(Self::vslot(v)), 0x6f);
                tmp
            }
        }
    }

    fn vdst(&self, v: u8, tmp: u8) -> u8 {
        self.vpin(v).unwrap_or(tmp)
    }

    fn vcommit(&mut self, v: u8, x: u8) {
        match self.vpin(v) {
            Some(p) => {
                if p != x {
                    self.vmov(p, x);
                }
            }
            None => self.a.vex(2, 1, false, 0, x, Rm::M(Self::vslot(v)), 0x7f),
        }
    }

    fn vmov(&mut self, d: u8, s: u8) {
        if d != s {
            self.a.vex(0, 1, false, 0, d, Rm::R(s), 0x28);
        }
    }

    fn zmark(&mut self, v: u8, lvl: u8) {
        let b = 1u32 << v;
        self.zd = if lvl >= 1 { self.zd | b } else { self.zd & !b };
        self.zs = if lvl >= 2 { self.zs | b } else { self.zs & !b };
    }

    fn zknown(&self, v: u8, ft: u8) -> bool {
        let b = 1u32 << v;
        if ft == 0 {
            self.zs & b != 0
        } else {
            (self.zd | self.zs) & b != 0
        }
    }

    fn vclean(&mut self, xa: u8, xn: u8, ft: u8) {
        if ft == 1 {
            self.a.vex(2, 1, false, 0, xa, Rm::R(xn), 0x7e);
        } else {
            self.a.vex(1, 1, false, 0, xn, Rm::R(RAX), 0x7e);
            self.a.vex(1, 1, false, 0, xa, Rm::R(RAX), 0x6e);
        }
    }

    fn vzero(&mut self, x: u8) {
        self.a.vex(1, 1, false, x, x, Rm::R(x), 0xef);
    }

    fn vx(&mut self, pp: u8, opc: u8, d: u8, v: u8, rm: Rm) {
        self.a.vex(pp, 1, false, v, d, rm, opc);
    }

    fn clean_upper(&mut self, x: u8, q: bool) {
        if !q {
            self.a.vex(2, 1, false, 0, x, Rm::R(x), 0x7e);
        }
    }

    pub(super) fn vinsn(&mut self, i: &Insn) {
        match *i {
            Insn::VLd { size, rt, rn, addr } => {
                let (m, wb) = self.amem(rn, addr);
                self.mark();
                let x = self.vdst(rt, X15);
                self.vload(size, x, m);
                self.vcommit(rt, x);
                self.zmark(rt, if size <= 2 { 2 } else if size == 3 { 1 } else { 0 });
                if let Some(o) = wb {
                    self.writeback(rn, o);
                }
            }
            Insn::VSt { size, rt, rn, addr } => {
                let (m, wb) = self.amem(rn, addr);
                self.mark();
                let x = self.vsrc(rt, X15);
                self.vstore(size, m, x);
                if let Some(o) = wb {
                    self.writeback(rn, o);
                }
            }
            Insn::VLdp { sz, rt, rt2, rn, off, mode } => {
                let base = self.src(rn, T2);
                let (disp, wb) = match mode {
                    Mode::Offset => (off, None),
                    Mode::Pre => (off, Some(off)),
                    Mode::Post => (0, Some(off)),
                };
                let m0 = self.finish_mem(Mem::b(base, disp as i32));
                let mut m1 = m0;
                m1.disp += 1 << sz;
                self.mark();
                let x1 = self.vdst(rt, X14);
                let x2 = self.vdst(rt2, X15);
                self.vload(sz, x1, m0);
                self.vload(sz, x2, m1);
                self.vcommit(rt, x1);
                self.vcommit(rt2, x2);
                let l = if sz <= 2 { 2 } else if sz == 3 { 1 } else { 0 };
                self.zmark(rt, l);
                self.zmark(rt2, l);
                if let Some(o) = wb {
                    self.writeback(rn, o);
                }
            }
            Insn::VStp { sz, rt, rt2, rn, off, mode } => {
                let base = self.src(rn, T2);
                let (disp, wb) = match mode {
                    Mode::Offset => (off, None),
                    Mode::Pre => (off, Some(off)),
                    Mode::Post => (0, Some(off)),
                };
                let m0 = self.finish_mem(Mem::b(base, disp as i32));
                let mut m1 = m0;
                m1.disp += 1 << sz;
                self.mark();
                let x1 = self.vsrc(rt, X14);
                self.vstore(sz, m0, x1);
                let x2 = self.vsrc(rt2, X15);
                self.vstore(sz, m1, x2);
                if let Some(o) = wb {
                    self.writeback(rn, o);
                }
            }
            Insn::Movi { q, rd, imm } => {
                let x = self.vdst(rd, X15);
                if imm == 0 {
                    self.vzero(x);
                } else if imm == u64::MAX && q {
                    self.a.vex(1, 1, false, x, x, Rm::R(x), 0x76);
                } else {
                    if imm <= u32::MAX as u64 && !q {
                        self.a.mov_ri(RAX, imm);
                        self.a.vex(1, 1, true, 0, x, Rm::R(RAX), 0x6e);
                    } else {
                        self.a.mov_ri(RAX, imm);
                        self.a.vex(1, 1, true, 0, x, Rm::R(RAX), 0x6e);
                        if q {
                            self.a.vex(1, 2, false, 0, x, Rm::R(x), 0x59);
                        }
                    }
                }
                self.vcommit(rd, x);
                self.zmark(rd, if q { 0 } else { 1 });
            }
            Insn::FmovI { ftype, rd, imm } => {
                let x = self.vdst(rd, X15);
                if imm == 0 {
                    self.vzero(x);
                } else {
                    self.a.mov_ri(RAX, imm);
                    if ftype == 0 {
                        self.a.vex(1, 1, false, 0, x, Rm::R(RAX), 0x6e);
                    } else {
                        self.a.vex(1, 1, true, 0, x, Rm::R(RAX), 0x6e);
                    }
                }
                self.vcommit(rd, x);
                self.zmark(rd, if ftype == 0 { 2 } else { 1 });
            }
            Insn::FmovR { ftype, rd, rn } => {
                let xn = self.vsrc(rn, X13);
                let x = self.vdst(rd, X15);
                if self.zknown(rn, ftype) && x != xn && ftype == 1 || ftype == 1 {
                    self.vclean(x, xn, 1);
                } else {
                    self.vclean(x, xn, ftype);
                }
                self.vcommit(rd, x);
                self.zmark(rd, if ftype == 0 { 2 } else { 1 });
            }
            Insn::FmovToG { sf, ftype, top, rd, rn } => {
                if rd == ZR {
                    return;
                }
                let xn = self.vsrc(rn, X13);
                let d = self.dst(rd, T0);
                if top {
                    self.a.vex(1, 3, true, 0, xn, Rm::R(d), 0x16);
                    self.a.b(1);
                } else if ftype == 1 || sf {
                    self.a.vex(1, 1, ftype == 1, 0, xn, Rm::R(d), 0x7e);
                } else {
                    self.a.vex(1, 1, false, 0, xn, Rm::R(d), 0x7e);
                }
                self.commit(rd, d);
            }
            Insn::FmovFromG { sf, ftype, top, rd, rn } => {
                let r = self.src(rn, T0);
                if top {
                    let x = self.vdst(rd, X15);
                    if self.vpin(rd).is_none() {
                        self.a.vex(2, 1, false, 0, x, Rm::M(Self::vslot(rd)), 0x6f);
                    }
                    self.a.vex(1, 3, true, x, x, Rm::R(r), 0x22);
                    self.a.b(1);
                    self.vcommit(rd, x);
                    self.zmark(rd, 0);
                } else {
                    let x = self.vdst(rd, X15);
                    self.a.vex(1, 1, ftype == 1 && sf, 0, x, Rm::R(r), 0x6e);
                    self.vcommit(rd, x);
                    self.zmark(rd, if ftype == 0 { 2 } else { 1 });
                }
            }
            Insn::Fp1 { ftype, op, rd, rn } => self.fp1(ftype, op, rd, rn),
            Insn::Fp2 { ftype, op, rd, rn, rm } => self.fp2(ftype, op, rd, rn, rm),
            Insn::Fp3 { ftype, op, rd, rn, rm, ra } => self.fp3(ftype, op, rd, rn, rm, ra),
            Insn::Fcmp { ftype, rn, rm, zero, .. } => self.fcmp(ftype, rn, rm, zero),
            Insn::Fcsel { ftype, rd, rn, rm, cond } => self.fcsel(ftype, rd, rn, rm, cond),
            Insn::Fcvt { from, to, rd, rn } => {
                let xm = self.vrm(rn);
                let x = self.vdst(rd, X15);
                let t = if x == X15 { X14 } else { x };
                self.vzero(X13);
                let _ = t;
                let acc = if self.vpin(rd).is_some() { x } else { X15 };
                if from == 0 {
                    self.vx(2, 0x5a, acc, X13, xm);
                } else {
                    self.vx(3, 0x5a, acc, X13, xm);
                }
                self.vcommit(rd, acc);
                self.zmark(rd, if to == 0 { 2 } else { 1 });
            }
            Insn::CvtIF { sf, ftype, rd, rn, signed } => {
                let acc = if self.vpin(rd).is_some() { self.vdst(rd, X15) } else { X15 };
                self.vzero(X13);
                let r = self.src(rn, T0);
                let pp = pp_of(ftype);
                if signed {
                    self.a.vex(pp, 1, sf, X13, acc, Rm::R(r), 0x2a);
                } else if !sf {
                    self.a.mov_rr(false, RAX, r);
                    self.a.vex(pp, 1, true, X13, acc, Rm::R(RAX), 0x2a);
                } else {
                    self.clobber();
                    self.a.mov_rr(true, RAX, r);
                    self.a.test_rr(true, RAX, RAX);
                    let j1 = self.a.jcc8(CC_S);
                    self.a.vex(pp, 1, true, X13, acc, Rm::R(RAX), 0x2a);
                    let j2 = self.a.jmp8();
                    self.a.patch8(j1);
                    self.a.mov_rr(true, RDX, RAX);
                    self.a.shift_i(Shf::Shr, true, RDX, 1);
                    self.a.alu_ri(Alu::And, false, RAX, 1);
                    self.a.alu_rr(Alu::Or, true, RDX, RAX);
                    self.a.vex(pp, 1, true, X13, acc, Rm::R(RDX), 0x2a);
                    self.a.vex(pp, 1, false, acc, acc, Rm::R(acc), 0x58);
                    self.a.patch8(j2);
                }
                self.vcommit(rd, acc);
                self.zmark(rd, if ftype == 0 { 2 } else { 1 });
            }
            Insn::CvtFI { sf, ftype, rd, rn, signed } => self.cvtfi(sf, ftype, rd, rn, signed),
            Insn::VLog { op, q, rd, rn, rm } => self.vlog(op, q, rd, rn, rm),
            Insn::VArith { sub, size, q, rd, rn, rm } => {
                let xn = self.vsrc(rn, X13);
                let xm = self.vrm(rm);
                let x = self.vdst(rd, X15);
                let opc = if sub { 0xf8 + size } else if size == 3 { 0xd4 } else { 0xfc + size };
                self.a.vex(1, 1, false, xn, x, xm, opc);
                self.clean_upper(x, q);
                self.vcommit(rd, x);
                self.zmark(rd, if q { 0 } else { 1 });
            }
            Insn::VDupG { q, size, rd, rn } => {
                let r = self.src(rn, T0);
                let x = self.vdst(rd, X15);
                self.a.vex(1, 1, size == 3, 0, x, Rm::R(r), 0x6e);
                let opc = [0x78u8, 0x79, 0x58, 0x59][size as usize];
                self.a.vex(1, 2, false, 0, x, Rm::R(x), opc);
                self.clean_upper(x, q);
                self.vcommit(rd, x);
                self.zmark(rd, if q { 0 } else { 1 });
            }
            Insn::Umov { sf: _, size, idx, rd, rn } => {
                if rd == ZR {
                    return;
                }
                let xn = self.vsrc(rn, X13);
                let d = self.dst(rd, T0);
                match size {
                    0 => {
                        self.a.vex(1, 3, false, 0, xn, Rm::R(d), 0x14);
                        self.a.b(idx);
                    }
                    1 => {
                        self.a.vex(1, 3, false, 0, xn, Rm::R(d), 0x15);
                        self.a.b(idx);
                    }
                    2 => {
                        self.a.vex(1, 3, false, 0, xn, Rm::R(d), 0x16);
                        self.a.b(idx);
                    }
                    _ => {
                        self.a.vex(1, 3, true, 0, xn, Rm::R(d), 0x16);
                        self.a.b(idx);
                    }
                }
                self.commit(rd, d);
            }
            Insn::InsG { size, idx, rd, rn } => {
                let r = self.src(rn, T0);
                let x = self.vdst(rd, X15);
                if self.vpin(rd).is_none() {
                    self.a.vex(2, 1, false, 0, x, Rm::M(Self::vslot(rd)), 0x6f);
                }
                match size {
                    0 => {
                        self.a.vex(1, 3, false, x, x, Rm::R(r), 0x20);
                        self.a.b(idx);
                    }
                    1 => {
                        self.a.vex(1, 1, false, x, x, Rm::R(r), 0xc4);
                        self.a.b(idx);
                    }
                    2 => {
                        self.a.vex(1, 3, false, x, x, Rm::R(r), 0x22);
                        self.a.b(idx);
                    }
                    _ => {
                        self.a.vex(1, 3, true, x, x, Rm::R(r), 0x22);
                        self.a.b(idx);
                    }
                }
                self.vcommit(rd, x);
                self.zmark(rd, 0);
            }
            _ => {}
        }
    }

    fn vload(&mut self, size: u8, x: u8, m: Mem) {
        match size {
            0 => {
                self.a.load(0, false, false, RAX, m);
                self.a.vex(1, 1, false, 0, x, Rm::R(RAX), 0x6e);
            }
            1 => {
                self.a.load(1, false, false, RAX, m);
                self.a.vex(1, 1, false, 0, x, Rm::R(RAX), 0x6e);
            }
            2 => self.a.vex(1, 1, false, 0, x, Rm::M(m), 0x6e),
            3 => self.a.vex(2, 1, false, 0, x, Rm::M(m), 0x7e),
            _ => self.a.vex(2, 1, false, 0, x, Rm::M(m), 0x6f),
        }
    }

    fn vstore(&mut self, size: u8, m: Mem, x: u8) {
        match size {
            0 => {
                self.a.vex(1, 3, false, 0, x, Rm::M(m), 0x14);
                self.a.b(0);
            }
            1 => {
                self.a.vex(1, 3, false, 0, x, Rm::M(m), 0x15);
                self.a.b(0);
            }
            2 => self.a.vex(1, 1, false, 0, x, Rm::M(m), 0x7e),
            3 => self.a.vex(1, 1, false, 0, x, Rm::M(m), 0xd6),
            _ => self.a.vex(2, 1, false, 0, x, Rm::M(m), 0x7f),
        }
    }

    fn vlog(&mut self, op: u8, q: bool, rd: u8, rn: u8, rm: u8) {
        let xn = self.vsrc(rn, X13);
        let xm_rm = self.vrm(rm);
        let x = self.vdst(rd, X15);
        match op {
            0 => self.vx(1, 0xdb, x, xn, xm_rm),
            1 => {
                let xm = self.vsrc(rm, X14);
                self.vx(1, 0xdf, x, xm, Rm::R(xn));
            }
            2 => {
                if rn == rm {
                    if let Some(p) = self.vpin(rn) {
                        self.vmov(x, p);
                    } else {
                        self.vmov(x, xn);
                    }
                } else {
                    self.vx(1, 0xeb, x, xn, xm_rm);
                }
            }
            3 => {
                let xm = self.vsrc(rm, X14);
                self.a.vex(1, 1, false, X15, X15, Rm::R(X15), 0x76);
                self.vx(1, 0xef, X15, X15, Rm::R(xm));
                self.vx(1, 0xeb, x, xn, Rm::R(X15));
            }
            4 => self.vx(1, 0xef, x, xn, xm_rm),
            _ => {
                let xd0 = self.vsrc(rd, X15);
                let xm = self.vsrc(rm, X14);
                let t = X13;
                let xnn = if xn == X13 { xn } else { xn };
                let _ = xnn;
                let xn2 = self.vsrc(rn, X13);
                match op {
                    5 => {
                        self.vx(1, 0xef, t, xn2, Rm::R(xm));
                        self.vx(1, 0xdb, t, t, Rm::R(xd0));
                        self.vx(1, 0xef, x, t, Rm::R(xm));
                    }
                    6 => {
                        self.vx(1, 0xef, t, xd0, Rm::R(xn2));
                        self.vx(1, 0xdb, t, t, Rm::R(xm));
                        self.vx(1, 0xef, x, xd0, Rm::R(t));
                    }
                    _ => {
                        self.vx(1, 0xef, t, xd0, Rm::R(xn2));
                        self.vx(1, 0xdf, t, xm, Rm::R(t));
                        self.vx(1, 0xef, x, xd0, Rm::R(t));
                    }
                }
            }
        }
        self.clean_upper(x, q);
        self.vcommit(rd, x);
        self.zmark(rd, if q { 0 } else { 1 });
    }

    fn nan_fix(&mut self, acc: u8, ft: u8) {
        self.clobber();
        self.a.vex(if ft == 0 { 0 } else { 1 }, 1, false, 0, acc, Rm::R(acc), 0x2e);
        let at = self.a.jcc32(CC_P) as u32;
        let back = self.a.hot.len() as u32;
        self.a.in_cold = true;
        let off = self.a.cold.len() as u32;
        self.fixes.push(Fix::Cold { at, to: off });
        if ft == 1 {
            self.a.vex(1, 1, true, 0, acc, Rm::R(RAX), 0x7e);
            self.a.mov_ri(RDX, 0xfff8_0000_0000_0000);
            self.a.alu_rr(Alu::Cmp, true, RAX, RDX);
            let j = self.a.jcc8(CC_NE);
            self.a.op(0, true, &[0x0f, 0xba], 6, Rm::R(RAX), false);
            self.a.b(63);
            self.a.vex(1, 1, true, 0, acc, Rm::R(RAX), 0x6e);
            self.a.patch8(j);
        } else {
            self.a.vex(1, 1, false, 0, acc, Rm::R(RAX), 0x7e);
            self.a.alu_ri(Alu::Cmp, false, RAX, 0xffc0_0000u32 as i32);
            let j = self.a.jcc8(CC_NE);
            self.a.op(0, false, &[0x0f, 0xba], 6, Rm::R(RAX), false);
            self.a.b(31);
            self.a.vex(1, 1, false, 0, acc, Rm::R(RAX), 0x6e);
            self.a.patch8(j);
        }
        let bat = self.a.jmp32() as u32;
        self.fixes.push(Fix::Hot { at: bat, to: back });
        self.a.in_cold = false;
    }

    fn fp1(&mut self, ft: u8, op: u8, rd: u8, rn: u8) {
        let acc = if self.vpin(rd).is_some() { self.vdst(rd, X15) } else { X15 };
        match op {
            3 => {
                let xm = self.vrm(rn);
                self.vzero(X13);
                self.vx(pp_of(ft), 0x51, acc, X13, xm);
                self.nan_fix(acc, ft);
            }
            _ => {
                self.clobber();
                let xn = self.vsrc(rn, X13);
                if ft == 1 {
                    self.a.vex(1, 1, true, 0, xn, Rm::R(RAX), 0x7e);
                    if op == 1 {
                        self.a.op(0, true, &[0x0f, 0xba], 6, Rm::R(RAX), false);
                    } else {
                        self.a.op(0, true, &[0x0f, 0xba], 7, Rm::R(RAX), false);
                    }
                    self.a.b(63);
                    self.a.vex(1, 1, true, 0, acc, Rm::R(RAX), 0x6e);
                } else {
                    self.a.vex(1, 1, false, 0, xn, Rm::R(RAX), 0x7e);
                    if op == 1 {
                        self.a.op(0, false, &[0x0f, 0xba], 6, Rm::R(RAX), false);
                    } else {
                        self.a.op(0, false, &[0x0f, 0xba], 7, Rm::R(RAX), false);
                    }
                    self.a.b(31);
                    self.a.vex(1, 1, false, 0, acc, Rm::R(RAX), 0x6e);
                }
            }
        }
        self.vcommit(rd, acc);
        self.zmark(rd, if ft == 0 { 2 } else { 1 });
    }

    fn fp2(&mut self, ft: u8, op: u8, rd: u8, rn: u8, rm: u8) {
        let opc = match op {
            0 => 0x59,
            1 => 0x5e,
            2 => 0x58,
            _ => 0x5c,
        };
        let xm = self.vrm(rm);
        let pm = self.vpin(rm);
        let pn = self.vpin(rn);
        // The result goes to a register that is neither input, so `arm_nan` can still read
        // both inputs after the operation.
        let xa = match self.vpin(rd) {
            Some(p) if Some(p) != pm && Some(p) != pn => p,
            _ => X15,
        };
        let xn = self.vsrc(rn, X13);
        self.vclean(xa, xn, ft);
        self.vx(pp_of(ft), opc, xa, xa, xm);
        self.arm_nan(xa, ft, &[(Rm::R(xn), false), (xm, false)], false);
        self.vcommit(rd, xa);
        self.zmark(rd, if ft == 0 { 2 } else { 1 });
    }

    fn fp3(&mut self, ft: u8, op: u8, rd: u8, rn: u8, rm: u8, ra: u8) {
        let opc = match op {
            0 => 0xb9,
            1 => 0xbd,
            2 => 0xbf,
            _ => 0xbb,
        };
        let xn = self.vsrc(rn, X13);
        let xm = self.vrm(rm);
        let xra = self.vsrc(ra, X14);
        let acc = X15;
        self.vclean(acc, xra, ft);
        self.a.vex(1, 2, ft == 1, xn, acc, xm, opc);
        // Arm computes FMSUB/FNMADD/FNMSUB as FPMulAdd with the addend and/or Vn negated, and
        // the negation applies to NaN operands too (o1 negates the addend, o0 != o1 negates Vn).
        let (neg_a, neg_n) = (op >= 2, op == 1 || op == 2);
        self.arm_nan(acc, ft, &[(Rm::R(xra), neg_a), (Rm::R(xn), neg_n), (xm, false)], true);
        self.vcommit(rd, acc);
        self.zmark(rd, if ft == 0 { 2 } else { 1 });
    }

    /// Loads the raw bits of a scalar FP operand into a general register.
    fn fp_bits(&mut self, r: u8, src: Rm, double: bool) {
        match src {
            Rm::R(x) => self.a.vex(1, 1, double, 0, x, Rm::R(r), 0x7e),
            Rm::M(m) => self.a.load(if double { 3 } else { 2 }, false, double, r, m),
        }
    }

    fn patch32_to(&mut self, at: usize, target: usize) {
        let rel = (target as i64 - (at as i64 + 4)) as i32;
        self.a.buf()[at..at + 4].copy_from_slice(&rel.to_le_bytes());
    }

    /// Gives a scalar FP result Arm's NaN semantics (FPCR.DN = 0, AH = 0).
    ///
    /// x86 and Arm agree on every non-NaN result, but not on NaNs: x86 returns the first NaN
    /// operand in its own operand order and a negative default NaN for invalid operations.
    /// Arm prefers signalling NaNs over quiet ones (in Arm operand order), applies operand
    /// negation to NaNs, and its default NaN is positive. For the multiply-add family it also
    /// returns the default NaN when the addend is a quiet NaN and the product is inf * 0.
    ///
    /// The hot path is one ucomis + jp on the result. Only a NaN result takes the cold path,
    /// which rebuilds the result from the original operands. `ops` are the Arm operands in
    /// priority order with their negation; for `muladd`, ops[0] is the addend and ops[1],
    /// ops[2] the factors. The operands must still hold their inputs.
    fn arm_nan(&mut self, acc: u8, ft: u8, ops: &[(Rm, bool)], muladd: bool) {
        self.clobber();
        let d = ft == 1;
        let (sign, quiet) = if d { (63u8, 51u8) } else { (31u8, 22u8) };
        // Bits << 1 (sign dropped): above `inf2` is a NaN; quiet bit then sits at quiet + 1.
        let inf2: u64 = if d { 0xffe0_0000_0000_0000 } else { 0xff00_0000 };
        let dn: u64 = if d { 0x7ff8_0000_0000_0000 } else { 0x7fc0_0000 };

        self.a.vex(if d { 1 } else { 0 }, 1, false, 0, acc, Rm::R(acc), 0x2e); // ucomis acc, acc
        let at = self.a.jcc32(CC_P) as u32;
        let back = self.a.hot.len() as u32;
        self.a.in_cold = true;
        let off = self.a.cold.len() as u32;
        self.fixes.push(Fix::Cold { at, to: off });

        self.a.mov_ri(RDX, inf2);
        self.a.alu_rr(Alu::Xor, false, T2, T2); // bit i set: operand i is a quiet NaN
        let mut to_sel: Vec<(usize, usize)> = Vec::new();
        let mut to_dn: Vec<usize> = Vec::new();

        // 1. Signalling NaNs win, in operand order.
        for (i, &(src, _)) in ops.iter().enumerate() {
            self.fp_bits(RAX, src, d);
            self.a.shift_i(Shf::Shl, d, RAX, 1);
            self.a.alu_rr(Alu::Cmp, d, RAX, RDX);
            let not_nan = self.a.jcc8(CC_BE);
            self.a.op(0, d, &[0x0f, 0xba], 4, Rm::R(RAX), false); // bt rax, quiet + 1
            self.a.b(quiet + 1);
            to_sel.push((self.a.jcc32(CC_AE), i)); // quiet bit clear: signalling
            self.a.alu_ri(Alu::Or, false, T2, 1 << i);
            self.a.patch8(not_nan);
        }
        // 2. Then quiet NaNs, in operand order.
        let mut addend_qnan = None;
        for i in 0..ops.len() {
            self.a.test_ri(false, T2, 1 << i);
            let p = self.a.jcc32(CC_NE);
            if muladd && i == 0 {
                addend_qnan = Some(p);
            } else {
                to_sel.push((p, i));
            }
        }
        // 3. No NaN operand: an invalid operation, which gives the default NaN.
        to_dn.push(self.a.jmp32());

        if let Some(p) = addend_qnan {
            // Quiet NaN addend, factors not NaN: inf * 0 still gives the default NaN.
            let here = self.a.pos();
            self.patch32_to(p, here);
            self.fp_bits(RAX, ops[1].0, d);
            self.a.shift_i(Shf::Shl, d, RAX, 1);
            self.fp_bits(T2, ops[2].0, d);
            self.a.shift_i(Shf::Shl, d, T2, 1);
            self.a.alu_rr(Alu::Cmp, d, RAX, RDX);
            let n_not_inf = self.a.jcc8(CC_NE);
            self.a.test_rr(d, T2, T2);
            to_dn.push(self.a.jcc32(CC_E)); // inf * 0
            self.a.patch8(n_not_inf);
            self.a.test_rr(d, RAX, RAX);
            to_sel.push((self.a.jcc32(CC_NE), 0)); // n not zero: addend wins
            self.a.alu_rr(Alu::Cmp, d, T2, RDX);
            to_dn.push(self.a.jcc32(CC_E)); // 0 * inf
            to_sel.push((self.a.jmp32(), 0));
        }

        // Selected operand: quieted, with its Arm negation applied.
        let mut to_store = Vec::new();
        let mut sel_at = vec![0usize; ops.len()];
        for (i, &(src, neg)) in ops.iter().enumerate() {
            sel_at[i] = self.a.pos();
            self.fp_bits(RAX, src, d);
            if neg {
                self.a.op(0, d, &[0x0f, 0xba], 7, Rm::R(RAX), false); // btc rax, sign
                self.a.b(sign);
            }
            self.a.op(0, d, &[0x0f, 0xba], 5, Rm::R(RAX), false); // bts rax, quiet
            self.a.b(quiet);
            to_store.push(self.a.jmp32());
        }
        let dn_at = self.a.pos();
        self.a.mov_ri(RAX, dn);
        let store_at = self.a.pos();
        self.a.vex(1, 1, d, 0, acc, Rm::R(RAX), 0x6e); // movd/movq acc, rax
        let bat = self.a.jmp32() as u32;
        self.fixes.push(Fix::Hot { at: bat, to: back });

        for (p, i) in to_sel {
            self.patch32_to(p, sel_at[i]);
        }
        for p in to_dn {
            self.patch32_to(p, dn_at);
        }
        for p in to_store {
            self.patch32_to(p, store_at);
        }
        self.a.in_cold = false;
    }

    fn fcmp(&mut self, ft: u8, rn: u8, rm: u8, zero: bool) {
        self.clobber();
        let xn = self.vsrc(rn, X13);
        let xm = if zero {
            self.vzero(X14);
            Rm::R(X14)
        } else {
            self.vrm(rm)
        };
        self.a.vex(if ft == 0 { 0 } else { 1 }, 1, false, 0, xn, xm, 0x2e);
        let jp = self.a.jcc8(CC_P);
        let jae = self.a.jcc8(CC_AE);
        let img_lt = (self.cfg.eflags_base as i32 & !0x8c1) | 0x80 | 1;
        self.a.push_i32(img_lt);
        self.a.popfq();
        let jend = self.a.jmp8();
        self.a.patch8(jp);
        let img_un = (self.cfg.eflags_base as i32 & !0x8c1) | 0x800;
        self.a.push_i32(img_un);
        self.a.popfq();
        self.a.patch8(jae);
        self.a.patch8(jend);
        self.set_flags(Fl::Sub);
    }

    fn fcsel(&mut self, ft: u8, rd: u8, rn: u8, rm: u8, cond: u8) {
        let acc = if self.vpin(rd).is_some() { self.vdst(rd, X15) } else { X15 };
        if cond >= 14 {
            let xn = self.vsrc(rn, X13);
            self.vclean(acc, xn, ft);
        } else {
            self.need_flags();
            let cc = self.cc_for(cond);
            let xn = self.vsrc(rn, X13);
            let xm = self.vsrc(rm, X14);
            let j1 = self.a.jcc8(cc ^ 1);
            self.vclean(acc, xn, ft);
            let j2 = self.a.jmp8();
            self.a.patch8(j1);
            self.vclean(acc, xm, ft);
            self.a.patch8(j2);
        }
        self.vcommit(rd, acc);
        self.zmark(rd, if ft == 0 { 2 } else { 1 });
    }

    fn cvtfi(&mut self, sf: bool, ft: u8, rd: u8, rn: u8, signed: bool) {
        if rd == ZR {
            return;
        }
        self.clobber();
        let xm = self.vrm(rn);
        let d = self.dst(rd, T0);
        let pp = pp_of(ft);
        let w64 = sf || !signed;
        self.a.vex(pp, 1, w64, 0, d, xm, 0x2c);
        let cmp_pp = if ft == 0 { 0 } else { 1 };
        let cc;
        if signed {
            self.a.alu_ri(Alu::Cmp, sf, d, 1);
            cc = CC_O;
        } else if !sf {
            self.a.mov_rr(true, T1, d);
            self.a.shift_i(Shf::Shr, true, T1, 32);
            cc = CC_NE;
        } else {
            self.a.test_rr(true, d, d);
            cc = CC_S;
        }
        let at = self.a.jcc32(cc) as u32;
        let back = self.a.hot.len() as u32;
        self.a.in_cold = true;
        let off = self.a.cold.len() as u32;
        self.fixes.push(Fix::Cold { at, to: off });
        let xn = self.vsrc(rn, X13);
        self.a.vex(cmp_pp, 1, false, 0, xn, Rm::R(xn), 0x2e);
        let jnan = self.a.jcc8(CC_P);
        self.vzero(X14);
        self.a.vex(cmp_pp, 1, false, 0, xn, Rm::R(X14), 0x2e);
        let jle = self.a.jcc8(CC_BE);
        if signed {
            self.a.mov_ri(d, if sf { i64::MAX as u64 } else { i32::MAX as u64 });
            let jb = self.a.jmp8();
            self.a.patch8(jnan);
            self.a.mov_ri(d, 0);
            self.a.patch8(jle);
            self.a.patch8(jb);
        } else if !sf {
            self.a.mov_ri(d, 0xffff_ffff);
            let jb = self.a.jmp8();
            self.a.patch8(jnan);
            self.a.patch8(jle);
            self.a.mov_ri(d, 0);
            self.a.patch8(jb);
        } else {
            let c = if ft == 0 { 0x5f00_0000u64 } else { 0x43e0_0000_0000_0000u64 };
            self.a.mov_ri(T1, c);
            self.a.vex(1, 1, ft == 1, 0, X14, Rm::R(T1), 0x6e);
            self.a.vex(pp_of(ft), 1, false, X14, X14, Rm::R(xn), 0x5c);
            self.a.vex(pp, 1, true, 0, T2, Rm::R(X14), 0x2c);
            self.a.test_rr(true, T2, T2);
            let jbig = self.a.jcc8(CC_S);
            self.a.mov_ri(T1, 1u64 << 63);
            self.a.alu_rr(Alu::Xor, true, T2, T1);
            self.a.mov_rr(true, d, T2);
            let j1 = self.a.jmp8();
            self.a.patch8(jbig);
            self.a.mov_ri(d, u64::MAX);
            let j2 = self.a.jmp8();
            self.a.patch8(jnan);
            self.a.patch8(jle);
            self.a.mov_ri(d, 0);
            self.a.patch8(j1);
            self.a.patch8(j2);
        }
        let bat = self.a.jmp32() as u32;
        self.fixes.push(Fix::Hot { at: bat, to: back });
        self.a.in_cold = false;
        self.commit(rd, d);
    }
}
