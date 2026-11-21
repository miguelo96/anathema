use crate::mem::{Bus, Ram};
use crate::mmu::Mmu;
use anathema_xlat::decode::{decode, Addr, AtomOp, Ext, ExcKind, Insn, Mode, ZR};
use anathema_xlat::run::Ctx;
use anathema_xlat::trans::{sysreg_id, Fetch};
use std::collections::HashMap;
use std::time::Instant;

pub struct Machine {
    pub ram: Ram,
    pub bus: Bus,
    pub mmu: Mmu,
    pub sys: HashMap<u16, u64>,
    pub counts: HashMap<String, usize>,
    pub started: Instant,
    pub events: usize,
    pub trace_budget: usize,
    pub flush: bool,
    pub elog: bool,
    pub from: usize,
    pub daif: u64,
    tlb: HashMap<u64, u64>,
}

pub enum Step {
    Next,
    Stop(String),
}

pub fn sysname(id: u16) -> String {
    let op0 = 2 | ((id >> 14) & 1);
    format!("S{}_{}_C{}_C{}_{}", op0, (id >> 11) & 7, (id >> 7) & 15, (id >> 3) & 15, id & 7)
}

fn rd(ctx: &Ctx, r: u8) -> u64 {
    match r {
        31 => 0,
        32 => ctx.x[31],
        r => ctx.x[r as usize],
    }
}

fn wr(ctx: &mut Ctx, r: u8, v: u64) {
    match r {
        31 => {}
        32 => ctx.x[31] = v,
        r => ctx.x[r as usize] = v,
    }
}

fn extend(v: u64, e: Ext) -> u64 {
    match e {
        Ext::Uxtb => v & 0xff,
        Ext::Uxth => v & 0xffff,
        Ext::Uxtw => v & 0xffff_ffff,
        Ext::Uxtx | Ext::Sxtx => v,
        Ext::Sxtb => v as u8 as i8 as i64 as u64,
        Ext::Sxth => v as u16 as i16 as i64 as u64,
        Ext::Sxtw => v as u32 as i32 as i64 as u64,
    }
}

fn mask(size: usize) -> u64 {
    if size >= 8 {
        u64::MAX
    } else {
        (1u64 << (size * 8)) - 1
    }
}

fn sext_to(v: u64, size: usize, bits64: bool) -> u64 {
    let shift = 64 - size as u32 * 8;
    let s = ((v << shift) as i64 >> shift) as u64;
    if bits64 {
        s
    } else {
        s & 0xffff_ffff
    }
}

impl Machine {
    pub fn new(ram: Ram) -> Machine {
        let mut sys = HashMap::new();
        sys.insert(sysreg_id(3, 0, 4, 2, 2), 0x4);
        sys.insert(sysreg_id(3, 0, 0, 0, 0), 0x612f_0000);
        sys.insert(sysreg_id(3, 0, 0, 0, 5), 0x8000_0000);
        sys.insert(sysreg_id(3, 3, 14, 0, 0), 24_000_000);
        Machine {
            ram,
            bus: Bus::new(),
            mmu: Mmu::default(),
            sys,
            counts: HashMap::new(),
            started: Instant::now(),
            events: 0,
            trace_budget: 400,
            flush: false,
            elog: std::env::var("RT_ELOG").is_ok(),
            from: std::env::var("RT_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX),
            daif: 0x3c0,
            tlb: HashMap::new(),
        }
    }

    pub fn translate(&mut self, va: u64) -> Result<u64, String> {
        if !self.mmu.enabled() {
            return Ok(va);
        }
        let page = va & !0x3fff;
        if let Some(pa) = self.tlb.get(&page) {
            return Ok(pa | (va & 0x3fff));
        }
        match self.mmu.translate(&self.ram, va) {
            Ok(pa) => {
                self.tlb.insert(page, pa & !0x3fff);
                Ok(pa)
            }
            Err(f) => Err(format!("translation fault at {:#x} (level {})", f.va, f.level)),
        }
    }

    pub fn flush_tlb(&mut self) {
        self.tlb.clear();
        self.flush = true;
    }

    pub fn load(&mut self, va: u64, size: usize) -> Result<u64, String> {
        if size == 16 {
            return Err("wide load".into());
        }
        let pa = self.translate(va)?;
        if self.ram.contains(pa, size as u64) {
            Ok(self.ram.read(pa, size))
        } else if pa < self.ram.base {
            Ok(self.bus.read(pa, size))
        } else {
            Err(format!("load from unmapped physical {:#x} (va {:#x})", pa, va))
        }
    }

    pub fn store(&mut self, va: u64, size: usize, v: u64) -> Result<(), String> {
        let pa = self.translate(va)?;
        if self.ram.contains(pa, size as u64) {
            self.ram.write(pa, size, v);
            Ok(())
        } else if pa < self.ram.base {
            self.bus.write(pa, size, v);
            Ok(())
        } else {
            Err(format!("store to unmapped physical {:#x} (va {:#x})", pa, va))
        }
    }

    pub fn fetch_word(&mut self, pc: u64) -> Option<u32> {
        let pa = self.translate(pc).ok()?;
        if self.ram.contains(pa, 4) {
            Some(self.ram.read(pa, 4) as u32)
        } else {
            None
        }
    }

    fn count(&mut self, key: String) {
        *self.counts.entry(key).or_default() += 1;
    }

    fn ea(&self, ctx: &Ctx, pc: u64, rn: u8, a: &Addr) -> (u64, Option<u64>) {
        match *a {
            Addr::Lit { off } => (pc.wrapping_add(off as u64), None),
            Addr::Imm { off, mode } => {
                let base = rd(ctx, rn);
                match mode {
                    Mode::Offset => (base.wrapping_add(off as u64), None),
                    Mode::Pre => {
                        let e = base.wrapping_add(off as u64);
                        (e, Some(e))
                    }
                    Mode::Post => (base, Some(base.wrapping_add(off as u64))),
                }
            }
            Addr::Reg { rm, ext, amt } => {
                let off = extend(rd(ctx, rm), ext) << amt;
                (rd(ctx, rn).wrapping_add(off), None)
            }
        }
    }

    fn pair_ea(&self, ctx: &Ctx, rn: u8, off: i64, mode: Mode) -> (u64, Option<u64>) {
        let base = rd(ctx, rn);
        match mode {
            Mode::Offset => (base.wrapping_add(off as u64), None),
            Mode::Pre => {
                let e = base.wrapping_add(off as u64);
                (e, Some(e))
            }
            Mode::Post => (base, Some(base.wrapping_add(off as u64))),
        }
    }

    fn qload(&mut self, ctx: &mut Ctx, rt: u8, va: u64, size: usize) -> Result<(), String> {
        let mut buf = [0u8; 16];
        let mut done = 0;
        while done < size {
            let chunk = if size - done >= 8 { 8 } else { size - done };
            let v = self.load(va + done as u64, chunk)?;
            buf[done..done + chunk].copy_from_slice(&v.to_le_bytes()[..chunk]);
            done += chunk;
        }
        ctx.q[rt as usize] = buf;
        Ok(())
    }

    fn qstore(&mut self, ctx: &Ctx, rt: u8, va: u64, size: usize) -> Result<(), String> {
        let buf = ctx.q[rt as usize];
        let mut done = 0;
        while done < size {
            let chunk = if size - done >= 8 { 8 } else { size - done };
            let mut b = [0u8; 8];
            b[..chunk].copy_from_slice(&buf[done..done + chunk]);
            self.store(va + done as u64, chunk, u64::from_le_bytes(b))?;
            done += chunk;
        }
        Ok(())
    }

    pub fn step(&mut self, ctx: &mut Ctx, runner_flush: &mut bool) -> Step {
        let pc = ctx.pc;
        let w = match self.fetch_word(pc) {
            Some(w) => w,
            None => return Step::Stop(format!("instruction fetch failed at {:#x}", pc)),
        };
        self.events += 1;
        let insn = decode(w);
        if self.elog {
            let mut fp = 0u64;
            for (i, v) in ctx.x.iter().enumerate() {
                fp ^= v.rotate_left(i as u32 * 2 + 1);
            }
            println!("E {} {:x} {:08x} {:016x} f{:x}", self.events, pc, w, fp, ctx.flags & 0x8c1);
        }
        let r = self.exec(ctx, pc, w, insn);
        if self.flush {
            self.flush = false;
            *runner_flush = true;
        }
        r
    }

    fn exec(&mut self, ctx: &mut Ctx, pc: u64, w: u32, insn: Insn) -> Step {
        let say = self.events <= self.trace_budget || self.events >= self.from;
        macro_rules! tryv {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => return Step::Stop(format!("{} at pc {:#x}", e, pc)),
                }
            };
        }
        match insn {
            Insn::Ld { size, sext, rt, rn, addr } => {
                let (ea, wb) = self.ea(ctx, pc, rn, &addr);
                let n = 1usize << size;
                let mut v = tryv!(self.load(ea, n));
                if sext == 1 {
                    v = sext_to(v, n, false);
                } else if sext == 2 {
                    v = sext_to(v, n, true);
                }
                wr(ctx, rt, v);
                if let Some(b) = wb {
                    wr(ctx, rn, b);
                }
            }
            Insn::St { size, rt, rn, addr } => {
                let (ea, wb) = self.ea(ctx, pc, rn, &addr);
                let n = 1usize << size;
                tryv!(self.store(ea, n, rd(ctx, rt) & mask(n)));
                if let Some(b) = wb {
                    wr(ctx, rn, b);
                }
            }
            Insn::Ldp { sz, sx, rt, rt2, rn, off, mode } => {
                let (ea, wb) = self.pair_ea(ctx, rn, off, mode);
                let n = 1usize << sz;
                let a = tryv!(self.load(ea, n));
                let b = tryv!(self.load(ea + n as u64, n));
                let (a, b) = if sx { (sext_to(a, 4, true), sext_to(b, 4, true)) } else { (a, b) };
                wr(ctx, rt, a);
                wr(ctx, rt2, b);
                if let Some(x) = wb {
                    wr(ctx, rn, x);
                }
            }
            Insn::Stp { sz, rt, rt2, rn, off, mode } => {
                let (ea, wb) = self.pair_ea(ctx, rn, off, mode);
                let n = 1usize << sz;
                tryv!(self.store(ea, n, rd(ctx, rt) & mask(n)));
                tryv!(self.store(ea + n as u64, n, rd(ctx, rt2) & mask(n)));
                if let Some(x) = wb {
                    wr(ctx, rn, x);
                }
            }
            Insn::Ldex { size, rt, rt2, rn, pair } => {
                let ea = rd(ctx, rn);
                let n = 1usize << size;
                let a = tryv!(self.load(ea, n));
                wr(ctx, rt, a);
                if pair {
                    let b = tryv!(self.load(ea + n as u64, n));
                    wr(ctx, rt2, b);
                }
            }
            Insn::Stex { size, rs, rt, rt2, rn, pair } => {
                let ea = rd(ctx, rn);
                let n = 1usize << size;
                tryv!(self.store(ea, n, rd(ctx, rt) & mask(n)));
                if pair {
                    tryv!(self.store(ea + n as u64, n, rd(ctx, rt2) & mask(n)));
                }
                wr(ctx, rs, 0);
            }
            Insn::Ldar { size, rt, rn } => {
                let n = 1usize << size;
                let v = tryv!(self.load(rd(ctx, rn), n));
                wr(ctx, rt, v);
            }
            Insn::Stlr { size, rt, rn } => {
                let n = 1usize << size;
                tryv!(self.store(rd(ctx, rn), n, rd(ctx, rt) & mask(n)));
            }
            Insn::Atomic { op, size, rs, rt, rn } => {
                let ea = rd(ctx, rn);
                let n = 1usize << size;
                let old = tryv!(self.load(ea, n));
                let s = rd(ctx, rs) & mask(n);
                let signed = |v: u64| sext_to(v, n, true) as i64;
                let new = match op {
                    AtomOp::Add => old.wrapping_add(s),
                    AtomOp::Clr => old & !s,
                    AtomOp::Eor => old ^ s,
                    AtomOp::Set => old | s,
                    AtomOp::Smax => {
                        if signed(old) >= signed(s) {
                            old
                        } else {
                            s
                        }
                    }
                    AtomOp::Smin => {
                        if signed(old) <= signed(s) {
                            old
                        } else {
                            s
                        }
                    }
                    AtomOp::Umax => old.max(s),
                    AtomOp::Umin => old.min(s),
                    AtomOp::Swp => s,
                };
                tryv!(self.store(ea, n, new & mask(n)));
                wr(ctx, rt, old);
            }
            Insn::Cas { size, rs, rt, rn } => {
                let ea = rd(ctx, rn);
                let n = 1usize << size;
                let old = tryv!(self.load(ea, n));
                if old == rd(ctx, rs) & mask(n) {
                    tryv!(self.store(ea, n, rd(ctx, rt) & mask(n)));
                }
                wr(ctx, rs, old);
            }
            Insn::VLd { size, rt, rn, addr } => {
                let (ea, wb) = self.ea(ctx, pc, rn, &addr);
                tryv!(self.qload(ctx, rt, ea, 1usize << size));
                if let Some(b) = wb {
                    wr(ctx, rn, b);
                }
            }
            Insn::VSt { size, rt, rn, addr } => {
                let (ea, wb) = self.ea(ctx, pc, rn, &addr);
                tryv!(self.qstore(ctx, rt, ea, 1usize << size));
                if let Some(b) = wb {
                    wr(ctx, rn, b);
                }
            }
            Insn::VLdp { sz, rt, rt2, rn, off, mode } => {
                let (ea, wb) = self.pair_ea(ctx, rn, off, mode);
                let n = 1usize << sz;
                tryv!(self.qload(ctx, rt, ea, n));
                tryv!(self.qload(ctx, rt2, ea + n as u64, n));
                if let Some(x) = wb {
                    wr(ctx, rn, x);
                }
            }
            Insn::VStp { sz, rt, rt2, rn, off, mode } => {
                let (ea, wb) = self.pair_ea(ctx, rn, off, mode);
                let n = 1usize << sz;
                tryv!(self.qstore(ctx, rt, ea, n));
                tryv!(self.qstore(ctx, rt2, ea + n as u64, n));
                if let Some(x) = wb {
                    wr(ctx, rn, x);
                }
            }
            Insn::Mrs { rt, id } => {
                let v = self.read_sys(id);
                if rt != ZR {
                    wr(ctx, rt, v);
                }
                self.count(format!("mrs {}", sysname(id)));
                if say {
                    println!("#{} {:#x} mrs x{} <- {} = {:#x}", self.events, pc, rt, sysname(id), v);
                }
            }
            Insn::Msr { rt, id } => {
                let v = rd(ctx, rt);
                self.write_sys(id, v);
                self.count(format!("msr {}", sysname(id)));
                if say {
                    println!("#{} {:#x} msr {} <- x{} = {:#x}", self.events, pc, sysname(id), rt, v);
                }
            }
            Insn::MsrImm { field, imm } => {
                if field == 30 {
                    self.daif |= (imm as u64 & 0xf) << 6;
                } else if field == 31 {
                    self.daif &= !((imm as u64 & 0xf) << 6);
                }
                self.count(format!("msr-imm {}", field));
                if say {
                    println!("#{} {:#x} msr-imm field {} imm {}", self.events, pc, field, imm);
                }
            }
            Insn::Sys { l, op1, crn, crm, op2, rt } => {
                self.count(format!("sys {} {} c{} c{} {}", if l { "sysl" } else { "sys" }, op1, crn, crm, op2));
                if crn == 8 {
                    self.flush_tlb();
                }
                if say {
                    println!("#{} {:#x} sys op1 {} c{} c{} op2 {} x{}", self.events, pc, op1, crn, crm, op2, rt);
                }
            }
            Insn::Exc { kind: ExcKind::Hvc | ExcKind::Smc, imm } => {
                let f = ctx.x[0];
                let (ret, name) = match f {
                    0x8400_0000 => (0x0001_0001, "psci version"),
                    0x8400_0006 => (2, "psci migrate info"),
                    0xc400_0003 | 0x8400_0003 => (u64::MAX, "psci cpu on"),
                    0x8400_0008 => return Step::Stop("psci system off".into()),
                    0x8400_0009 => return Step::Stop("psci system reset".into()),
                    f if f & 0xffff_ff00 == 0xc100_0000 => (0, "vm hypervisor service call"),
                    _ => (u64::MAX, "unknown call"),
                };
                self.count(format!("hvc/smc {:#x} {}", f, name));
                if say || name == "unknown call" {
                    println!("#{} {:#x} {} #{} function {:#x} args {:#x} {:#x} {:#x} -> {} ({:#x})", self.events, pc, if matches!(insn, Insn::Exc { kind: ExcKind::Hvc, .. }) { "hvc" } else { "smc" }, imm, f, ctx.x[1], ctx.x[2], ctx.x[3], name, ret);
                }
                ctx.x[0] = ret;
            }
            Insn::Hint { op } if op == 2 || op == 3 => {
                return Step::Stop(format!("wait for event/interrupt at {:#x}", pc));
            }
            Insn::Hint { .. } => {}
            Insn::Unsup(_) if crate::simd::fallback(ctx, w) => {}
            Insn::Unsup(_) if w & 0xbe00_0000 == 0x0c00_0000 => tryv!(self.struct_ldst(ctx, w)),
            _ => return Step::Stop(format!("unhandled instruction {:08x} ({}) at {:#x}", w, insn.name(), pc)),
        }
        ctx.pc = pc + 4;
        Step::Next
    }


    fn struct_ldst(&mut self, ctx: &mut Ctx, w: u32) -> Result<(), String> {
        let q = (w >> 30) & 1;
        let single = (w >> 24) & 1 == 1;
        let post = (w >> 23) & 1 == 1;
        let load = (w >> 22) & 1 == 1;
        let rm = ((w >> 16) & 31) as u8;
        let rn = ((w >> 5) & 31) as u8;
        let rt = (w & 31) as usize;
        let base_reg = if rn == 31 { 32 } else { rn };
        let base = rd(ctx, base_reg);
        let datasize = if q == 1 { 16usize } else { 8 };
        let mut addr = base;
        let mut total = 0usize;
        if !single {
            let opcode = (w >> 12) & 0xf;
            let size = ((w >> 10) & 3) as usize;
            let esize = 1usize << size;
            let (rpt, selem) = match opcode {
                0x0 => (1, 4),
                0x2 => (4, 1),
                0x4 => (1, 3),
                0x6 => (3, 1),
                0x7 => (1, 1),
                0x8 => (1, 2),
                0xa => (2, 1),
                _ => return Err(format!("unsupported structure load/store {:08x}", w)),
            };
            if esize == 8 && q == 0 && selem > 1 {
                return Err(format!("reserved structure load/store {:08x}", w));
            }
            let elems = datasize / esize;
            for r in 0..rpt {
                for e in 0..elems {
                    for sidx in 0..selem {
                        let reg = (rt + r + sidx) % 32;
                        if load {
                            let v = self.load(addr, esize)?;
                            if e == 0 && sidx == 0 || true {
                                let mut qv = if e == 0 { [0u8; 16] } else { ctx.q[reg] };
                                qv[e * esize..(e + 1) * esize].copy_from_slice(&v.to_le_bytes()[..esize]);
                                ctx.q[reg] = qv;
                            }
                        } else {
                            let mut b = [0u8; 8];
                            b[..esize].copy_from_slice(&ctx.q[reg][e * esize..(e + 1) * esize]);
                            self.store(addr, esize, u64::from_le_bytes(b))?;
                        }
                        addr += esize as u64;
                        total += esize;
                    }
                }
            }
        } else {
            let opcode = (w >> 13) & 7;
            let r = (w >> 21) & 1;
            let s_bit = (w >> 12) & 1;
            let size = ((w >> 10) & 3) as usize;
            let scale = opcode >> 1;
            let selem = (((opcode & 1) << 1) | r) as usize + 1;
            if scale == 3 {
                let esize = 1usize << size;
                for sidx in 0..selem {
                    let reg = (rt + sidx) % 32;
                    let v = self.load(addr, esize)?;
                    let mut qv = [0u8; 16];
                    for i in 0..datasize / esize {
                        qv[i * esize..(i + 1) * esize].copy_from_slice(&v.to_le_bytes()[..esize]);
                    }
                    ctx.q[reg] = qv;
                    addr += esize as u64;
                    total += esize;
                }
            } else {
                let (esize, index) = match scale {
                    0 => (1usize, ((q << 3) | (s_bit << 2) | size as u32) as usize),
                    1 => (2usize, ((q << 2) | (s_bit << 1) | (size as u32 >> 1)) as usize),
                    _ => {
                        if size & 1 == 0 {
                            (4usize, ((q << 1) | s_bit) as usize)
                        } else {
                            (8usize, q as usize)
                        }
                    }
                };
                for sidx in 0..selem {
                    let reg = (rt + sidx) % 32;
                    if load {
                        let v = self.load(addr, esize)?;
                        let mut qv = ctx.q[reg];
                        qv[index * esize..(index + 1) * esize].copy_from_slice(&v.to_le_bytes()[..esize]);
                        ctx.q[reg] = qv;
                    } else {
                        let mut b = [0u8; 8];
                        b[..esize].copy_from_slice(&ctx.q[reg][index * esize..(index + 1) * esize]);
                        self.store(addr, esize, u64::from_le_bytes(b))?;
                    }
                    addr += esize as u64;
                    total += esize;
                }
            }
        }
        if post {
            let off = if rm == 31 { total as u64 } else { rd(ctx, rm) };
            wr(ctx, base_reg, base.wrapping_add(off));
        }
        Ok(())
    }

    fn read_sys(&mut self, id: u16) -> u64 {
        if id == sysreg_id(3, 3, 4, 2, 1) {
            return self.daif;
        }
        if id == sysreg_id(3, 3, 14, 0, 1) || id == sysreg_id(3, 3, 14, 0, 2) {
            return self.started.elapsed().as_nanos() as u64 * 24 / 1000;
        }
        self.sys.get(&id).copied().unwrap_or(0)
    }

    fn write_sys(&mut self, id: u16, v: u64) {
        self.sys.insert(id, v);
        if id == sysreg_id(3, 3, 4, 2, 1) {
            self.daif = v & 0x3c0;
        }
        if id == sysreg_id(3, 0, 1, 0, 0) {
            self.mmu.sctlr = v;
            self.flush_tlb();
        } else if id == sysreg_id(3, 0, 2, 0, 2) {
            self.mmu.tcr = v;
            self.flush_tlb();
        } else if id == sysreg_id(3, 0, 2, 0, 0) {
            self.mmu.ttbr0 = v;
            self.flush_tlb();
        } else if id == sysreg_id(3, 0, 2, 0, 1) {
            self.mmu.ttbr1 = v;
            self.flush_tlb();
        }
    }
}

pub struct Fetcher<'a> {
    pub m: std::cell::RefCell<&'a mut Machine>,
}

impl<'a> Fetch for Fetcher<'a> {
    fn word(&self, pc: u64) -> Option<u32> {
        if pc & 3 != 0 {
            return None;
        }
        self.m.borrow_mut().fetch_word(pc)
    }
}
