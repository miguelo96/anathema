use super::*;

impl<'a> Tx<'a> {
    fn finish_mem(&mut self, m: Mem) -> Mem {
        match self.cfg.mem {
            MemModel::Identity => m,
            MemModel::Biased => {
                if m.index == NOIDX {
                    Mem { base: m.base, index: R14, scale: 0, disp: m.disp }
                } else {
                    self.a.lea(true, T2, m);
                    Mem::bi(T2, R14, 0, 0)
                }
            }
            MemModel::Checked => {
                self.a.lea(true, T2, m);
                self.a.alu_rm(Alu::Cmp, true, T2, ctxm(OFF_LIMIT));
                let s = self.stub(self.pc, Shared::Interp, true);
                let at = self.a.jcc32(CC_B) as u32;
                self.fixes.push(Fix::Cold { at, to: s });
                Mem::b(T2, 0)
            }
        }
    }

    fn amem(&mut self, rn: u8, a: Addr) -> (Mem, Option<i64>) {
        match a {
            Addr::Imm { off, mode } => {
                let base = self.src(rn, T2);
                let (disp, wb) = match mode {
                    Mode::Offset => (off, None),
                    Mode::Pre => (off, Some(off)),
                    Mode::Post => (0, Some(off)),
                };
                let m = self.finish_mem(Mem::b(base, disp as i32));
                (m, wb)
            }
            Addr::Reg { rm, ext, amt } => {
                let base = self.src(rn, T2);
                let m = match ext {
                    Ext::Uxtw => {
                        let s = self.src(rm, T1);
                        self.a.mov_rr(false, T1, s);
                        Mem::bi(base, T1, amt, 0)
                    }
                    Ext::Sxtw => {
                        let s = self.src(rm, T1);
                        self.a.movsx_rr(2, true, T1, s);
                        Mem::bi(base, T1, amt, 0)
                    }
                    _ => {
                        if rm == ZR {
                            Mem::b(base, 0)
                        } else {
                            let s = self.src(rm, T1);
                            Mem::bi(base, s, amt, 0)
                        }
                    }
                };
                (self.finish_mem(m), None)
            }
            Addr::Lit { off } => {
                let abs = self.pc.wrapping_add(off as u64);
                self.a.mov_ri(T2, abs);
                (self.finish_mem(Mem::b(T2, 0)), None)
            }
        }
    }

    fn writeback(&mut self, rn: u8, off: i64) {
        match self.pin(rn) {
            Some(h) => self.a.lea(true, h, Mem::b(h, off as i32)),
            None => {
                self.a.op(0, true, &[0x8b], T2, Rm::M(gslot(rn)), false);
                self.a.lea(true, T2, Mem::b(T2, off as i32));
                self.a.store(3, gslot(rn), T2);
            }
        }
    }

    pub(super) fn ld(&mut self, size: u8, sext: u8, rt: u8, rn: u8, addr: Addr) {
        let (m, wb) = self.amem(rn, addr);
        self.mark();
        let d = if rt == ZR { T0 } else { self.dst(rt, T0) };
        self.a.load(size, sext != 0, sext == 2, d, m);
        if rt != ZR {
            self.commit(rt, d);
        }
        if let Some(o) = wb {
            self.writeback(rn, o);
        }
    }

    pub(super) fn st(&mut self, size: u8, rt: u8, rn: u8, addr: Addr) {
        let (m, wb) = self.amem(rn, addr);
        self.mark();
        if rt == ZR {
            self.a.store_i(size, m, 0);
        } else {
            let r = self.src(rt, T0);
            self.a.store(size, m, r);
        }
        if let Some(o) = wb {
            self.writeback(rn, o);
        }
    }

    pub(super) fn ldp(&mut self, sz: u8, sx: bool, rt: u8, rt2: u8, rn: u8, off: i64, mode: Mode) {
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
        let d1 = if rt == ZR { T0 } else { self.dst(rt, T0) };
        let d2 = if rt2 == ZR { T1 } else { self.dst(rt2, T1) };
        let swap = rt != ZR && self.pin(rt) == Some(base) || m0.index == d1;
        let (sizec, signed, w64) = if sx { (2, true, true) } else { (sz, false, sz == 3) };
        let _ = w64;
        if swap {
            self.a.load(sizec, signed, true, d2, m1);
            self.a.load(sizec, signed, true, d1, m0);
        } else {
            self.a.load(sizec, signed, true, d1, m0);
            self.a.load(sizec, signed, true, d2, m1);
        }
        if rt != ZR {
            self.commit(rt, d1);
        }
        if rt2 != ZR {
            self.commit(rt2, d2);
        }
        if let Some(o) = wb {
            self.writeback(rn, o);
        }
    }

    pub(super) fn stp(&mut self, sz: u8, rt: u8, rt2: u8, rn: u8, off: i64, mode: Mode) {
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
        if rt == ZR {
            self.a.store_i(sz, m0, 0);
        } else {
            let r = self.src(rt, T0);
            self.a.store(sz, m0, r);
        }
        if rt2 == ZR {
            self.a.store_i(sz, m1, 0);
        } else {
            let r = self.src(rt2, T1);
            self.a.store(sz, m1, r);
        }
        if let Some(o) = wb {
            self.writeback(rn, o);
        }
    }

    pub(super) fn ldex(&mut self, size: u8, rt: u8, rn: u8) {
        let base = self.src(rn, T2);
        let m = self.finish_mem(Mem::b(base, 0));
        self.mark();
        let d = if rt == ZR { T0 } else { self.dst(rt, T0) };
        self.a.load(size, false, size == 3, d, m);
        self.a.store(3, ctxm(OFF_EXADDR), base);
        self.a.store(3, ctxm(OFF_EXVAL), d);
        if rt != ZR {
            self.commit(rt, d);
        }
    }

    pub(super) fn stex(&mut self, size: u8, rs: u8, rt: u8, rn: u8) {
        self.clobber();
        let base = self.src(rn, T2);
        let m = self.finish_mem(Mem::b(base, 0));
        self.a.op(0, true, &[0x8b], RAX, Rm::M(ctxm(OFF_EXVAL)), false);
        let nv = if rt == ZR {
            self.a.mov_ri(T1, 0);
            T1
        } else {
            self.src(rt, T1)
        };
        self.a.alu_mr(Alu::Cmp, true, ctxm(OFF_EXADDR), base);
        let j = self.a.jcc8(CC_NE);
        self.mark();
        self.a.cmpxchg(size, m, nv);
        self.a.patch8(j);
        let d = if rs == ZR { T1 } else { self.dst(rs, T1) };
        self.a.setcc(CC_NE, d);
        self.a.store_i(3, ctxm(OFF_EXADDR), -1);
        self.a.movzx_rr(0, d, d);
        if rs != ZR {
            self.commit(rs, d);
        }
    }

    pub(super) fn atomic(&mut self, op: AtomOp, size: u8, rs: u8, rt: u8, rn: u8) {
        self.clobber();
        let base = self.src(rn, T2);
        let m = self.finish_mem(Mem::b(base, 0));
        let w = size == 3;
        match op {
            AtomOp::Add | AtomOp::Swp => {
                self.load_g(T1, rs);
                self.mark();
                if op == AtomOp::Add {
                    if rt == ZR {
                        self.a.lock();
                        match size {
                            0 => self.a.op(0, false, &[0x00], T1, Rm::M(m), (4..8).contains(&T1)),
                            1 => self.a.op(0x66, false, &[0x01], T1, Rm::M(m), false),
                            2 => self.a.op(0, false, &[0x01], T1, Rm::M(m), false),
                            _ => self.a.op(0, true, &[0x01], T1, Rm::M(m), false),
                        }
                        return;
                    }
                    self.a.xadd(size, m, T1);
                } else {
                    self.a.xchg_mr(size, m, T1);
                }
                if rt != ZR {
                    if size < 2 {
                        self.a.movzx_rr(size, T1, T1);
                    } else if size == 2 {
                        self.a.mov_rr(false, T1, T1);
                    }
                    let d = self.dst(rt, T1);
                    if d != T1 {
                        self.a.mov_rr(true, d, T1);
                    }
                    self.commit(rt, d);
                }
            }
            _ => {
                self.mark();
                self.a.load(size, false, w, RAX, m);
                let top = self.a.pos();
                match op {
                    AtomOp::Clr => {
                        self.load_g(T1, rs);
                        self.a.not(true, T1);
                        self.a.alu_rr(Alu::And, true, T1, RAX);
                    }
                    AtomOp::Set | AtomOp::Eor => {
                        self.a.mov_rr(true, T1, RAX);
                        let b = self.bb(B::G(rs), true);
                        let b = match b {
                            Bb::Imm(_) => {
                                self.a.mov_ri(T1, 0);
                                self.a.mov_rr(true, T1, RAX);
                                Bb::Imm(0)
                            }
                            x => x,
                        };
                        self.alu_b(if op == AtomOp::Set { Alu::Or } else { Alu::Xor }, true, T1, b);
                    }
                    _ => {
                        let b = self.bb(B::G(rs), true);
                        let (a1, cc) = match op {
                            AtomOp::Smax => (Alu::Cmp, CC_L),
                            AtomOp::Smin => (Alu::Cmp, CC_G),
                            AtomOp::Umax => (Alu::Cmp, CC_B),
                            _ => (Alu::Cmp, CC_A),
                        };
                        let sz64 = size == 3;
                        let _ = a1;
                        match b {
                            Bb::Reg(r) => self.a.alu_rr(Alu::Cmp, sz64, RAX, r),
                            Bb::Slot(mm) => self.a.alu_rm(Alu::Cmp, sz64, RAX, mm),
                            Bb::Imm(i) => self.a.alu_ri(Alu::Cmp, sz64, RAX, i),
                        }
                        self.a.mov_rr(true, T1, RAX);
                        match b {
                            Bb::Reg(r) => self.a.cmovcc(cc, sz64, T1, r),
                            Bb::Slot(mm) => self.a.cmovcc_m(cc, sz64, T1, mm),
                            Bb::Imm(_) => {}
                        }
                    }
                }
                self.a.cmpxchg(size, m, T1);
                let j = self.a.jcc8(CC_NE);
                let back = (top as i64 - (self.a.pos() as i64)) as i8;
                self.a.buf()[j] = back as u8;
                if rt != ZR {
                    let d = self.dst(rt, RAX);
                    if d != RAX {
                        self.a.mov_rr(true, d, RAX);
                    }
                    self.commit(rt, d);
                }
            }
        }
    }

    pub(super) fn cas(&mut self, size: u8, rs: u8, rt: u8, rn: u8) {
        self.clobber();
        let base = self.src(rn, T2);
        let m = self.finish_mem(Mem::b(base, 0));
        self.load_g(RAX, rs);
        match size {
            0 | 1 => self.a.movzx_rr(size, RAX, RAX),
            2 => self.a.mov_rr(false, RAX, RAX),
            _ => {}
        }
        let nv = if rt == ZR {
            self.a.mov_ri(T1, 0);
            T1
        } else {
            self.src(rt, T1)
        };
        self.mark();
        self.a.cmpxchg(size, m, nv);
        if rs != ZR {
            let d = self.dst(rs, RAX);
            if d != RAX {
                self.a.mov_rr(true, d, RAX);
            }
            self.commit(rs, d);
        }
    }
}
