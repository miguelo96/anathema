use crate::trans::*;
use crate::x86::*;
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone)]
pub struct Ctx {
    pub x: [u64; 32],
    pub pc: u64,
    pub flags: u64,
    pub exval: u64,
    pub exaddr: u64,
    pub limit: u64,
    pub sys: [u64; SYS_SLOTS],
}

impl Default for Ctx {
    fn default() -> Ctx {
        Ctx { x: [0; 32], pc: 0, flags: 0, exval: 0, exaddr: u64::MAX, limit: 0, sys: [0; SYS_SLOTS] }
    }
}

extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
}

pub struct Region {
    pub ptr: *mut u8,
    pub len: usize,
}

impl Region {
    pub fn fixed(addr: usize, len: usize, exec: bool) -> Option<Region> {
        let prot = if exec { 7 } else { 3 };
        let p = unsafe { mmap(addr as *mut u8, len, prot, 0x2 | 0x20 | 0x100000, -1, 0) };
        if p as isize == -1 || p as usize != addr {
            return None;
        }
        Some(Region { ptr: p, len })
    }

    pub fn anon(len: usize) -> Region {
        let p = unsafe { mmap(std::ptr::null_mut(), len, 7, 0x2 | 0x20, -1, 0) };
        assert!(p as isize != -1);
        Region { ptr: p, len }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            munmap(self.ptr, self.len);
        }
    }
}

pub fn flags_to_nzcv(img: u64) -> u64 {
    let n = (img >> 7) & 1;
    let z = (img >> 6) & 1;
    let c = (!img) & 1;
    let v = (img >> 11) & 1;
    (n << 31) | (z << 30) | (c << 29) | (v << 28)
}

pub fn nzcv_to_flags(nzcv: u64, base: u64) -> u64 {
    let mut img = base & !0x8c1;
    if nzcv & (1 << 31) != 0 {
        img |= 0x80;
    }
    if nzcv & (1 << 30) != 0 {
        img |= 0x40;
    }
    if nzcv & (1 << 29) == 0 {
        img |= 1;
    }
    if nzcv & (1 << 28) != 0 {
        img |= 0x800;
    }
    img
}

pub struct Runner {
    pub cfg: Config,
    region: Region,
    used: usize,
    entry: usize,
    exit_dispatch: usize,
    exit_indirect: usize,
    exit_interp: usize,
    shared_end: usize,
    cache: HashMap<u64, usize>,
}

pub struct Words<'a> {
    pub base: u64,
    pub words: &'a [u32],
}

impl<'a> Fetch for Words<'a> {
    fn word(&self, pc: u64) -> Option<u32> {
        if pc < self.base || pc & 3 != 0 {
            return None;
        }
        self.words.get(((pc - self.base) / 4) as usize).copied()
    }
}

impl Runner {
    pub fn new(cfg: Config) -> Runner {
        let region = Region::anon(1 << 22);
        let mut r = Runner {
            cfg,
            region,
            used: 0,
            entry: 0,
            exit_dispatch: 0,
            exit_indirect: 0,
            exit_interp: 0,
            shared_end: 0,
            cache: HashMap::new(),
        };
        r.build_shared();
        r
    }

    fn put(&mut self, bytes: &[u8]) -> usize {
        let at = self.used;
        assert!(at + bytes.len() < self.region.len);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.region.ptr.add(at), bytes.len());
        }
        self.used += bytes.len();
        at
    }

    fn build_shared(&mut self) {
        let pins = self.cfg.pinned();
        let mut a = Asm::new();
        for r in [RBX, RBP, R12, R13, R14, R15] {
            a.push_r(r);
        }
        a.lea(true, R15, Mem::b(RDI, 128));
        a.mov_rr(true, RAX, RSI);
        for (g, h) in &pins {
            let idx = if *g == 32 { 31 } else { *g as i32 };
            a.op(0, true, &[0x8b], *h, Rm::M(Mem::b(R15, idx * 8 - 128)), false);
        }
        if self.cfg.mem == MemModel::Biased {
            a.mov_ri(R14, 0);
        }
        a.jmp_rm(Rm::R(RAX));
        self.entry = self.put(&a.hot);
        let mk = |status: u32, add_base: bool, base: u64| -> Vec<u8> {
            let mut a = Asm::new();
            if add_base {
                a.mov_ri64(RDX, base);
                a.alu_rr(Alu::Add, true, RAX, RDX);
            }
            a.store(3, ctxm(OFF_PC), RAX);
            for (g, h) in &pins {
                let idx = if *g == 32 { 31 } else { *g as i32 };
                a.store(3, Mem::b(R15, idx * 8 - 128), *h);
            }
            a.mov_ri(RAX, status as u64);
            for r in [R15, R14, R13, R12, RBP, RBX] {
                a.pop_r(r);
            }
            a.ret();
            a.hot
        };
        let b = self.cfg.pc_base;
        let d = mk(0, true, b);
        self.exit_dispatch = self.put(&d);
        let i = mk(0, false, 0);
        self.exit_indirect = self.put(&i);
        let n = mk(2, true, b);
        self.exit_interp = self.put(&n);
        self.shared_end = self.used;
    }

    pub fn reset_cache(&mut self) {
        self.cache.clear();
        self.used = self.shared_end;
    }

    pub fn compile(&mut self, f: &dyn Fetch, pc: u64) -> Result<usize, String> {
        if let Some(&o) = self.cache.get(&pc) {
            return Ok(o);
        }
        let t = translate(&self.cfg, f, pc);
        if let Some((w, n)) = t.refused {
            return Err(format!("refused {:08x} {}", w, n));
        }
        let hot_base = self.used;
        let cold_base = hot_base + t.hot.len();
        let (d, i, n) = (self.exit_dispatch, self.exit_indirect, self.exit_interp);
        let (h, c) = t.layout(hot_base, cold_base, &|s| match s {
            Shared::Dispatch => d,
            Shared::Indirect => i,
            Shared::Interp => n,
        });
        self.put(&h);
        self.put(&c);
        self.cache.insert(pc, hot_base);
        Ok(hot_base)
    }

    pub fn run(&mut self, ctx: &mut Ctx, f: &dyn Fetch, end_pc: u64, budget: usize) -> Result<(), String> {
        for _ in 0..budget {
            let pc = ctx.pc;
            if pc == end_pc {
                return Ok(());
            }
            let off = self.compile(f, pc)?;
            let entry: extern "sysv64" fn(*mut Ctx, *const u8) -> u64 =
                unsafe { std::mem::transmute(self.region.ptr.add(self.entry)) };
            let st = entry(ctx as *mut Ctx, unsafe { self.region.ptr.add(off) });
            if st != 0 {
                return Err(format!("interp needed at {:x}", ctx.pc));
            }
        }
        Err("budget".into())
    }
}
