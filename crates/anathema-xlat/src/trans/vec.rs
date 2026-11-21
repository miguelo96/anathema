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

    fn fp1(&mut self, ft: u8, op: u8, rd: u8, rn: u8) {
        let acc = if self.vpin(rd).is_some() { self.vdst(rd, X15) } else { X15 };
        match op {
            3 => {
                let xm = self.vrm(rn);
                self.vzero(X13);
                self.vx(pp_of(ft), 0x51, acc, X13, xm);
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
        let xa = match self.vpin(rd) {
            Some(p) if Some(p) != pm || rn == rm => p,
            _ => X15,
        };
        let xn = self.vsrc(rn, X13);
        if xa != xn || !self.zknown(rn, ft) {
            self.vclean(xa, xn, ft);
        }
        let xm = if xa == xn && rn == rm { Rm::R(xa) } else { xm };
        self.vx(pp_of(ft), opc, xa, xa, xm);
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
        self.vcommit(rd, acc);
        self.zmark(rd, if ft == 0 { 2 } else { 1 });
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
