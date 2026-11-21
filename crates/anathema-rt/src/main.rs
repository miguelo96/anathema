mod macho;

use anathema_xlat::decode::{decode, ExcKind, Insn};
use anathema_xlat::run::{Ctx, Region, Runner, Stop};
use anathema_xlat::trans::{sysreg_id, Config, Fetch, MemModel};
use std::collections::HashMap;

const HOST_BASE: u64 = 0x2000_0000_0000;
const PBASE: u64 = 0x8_2000_0000;
const RAM_BASE: u64 = 0x8_0000_0000;
const RAM_SIZE: u64 = 0x2_0000_0000;
const SCRATCH: u64 = 0x80_0000;

static mut CTX_PTR: *const Ctx = std::ptr::null();
static mut BIAS: u64 = 0;

#[repr(C)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: i32,
    restorer: usize,
}

extern "C" {
    fn sigaction(sig: i32, act: *const SigAction, old: *mut SigAction) -> i32;
}

extern "C" fn on_segv(_sig: i32, info: *const u8, _uc: *mut u8) {
    unsafe {
        let addr = *(info.add(16) as *const u64);
        let pc = if CTX_PTR.is_null() { 0 } else { (*CTX_PTR).pc };
        eprintln!("host fault at {:#x} (guest {:#x}) in block starting at guest pc {:#x}", addr, addr.wrapping_sub(BIAS), pc);
        std::process::exit(99);
    }
}

fn install_segv() {
    let act = SigAction { handler: on_segv as usize, mask: [0; 16], flags: 4, restorer: 0 };
    unsafe {
        sigaction(11, &act, std::ptr::null_mut());
        sigaction(7, &act, std::ptr::null_mut());
    }
}

struct Guest {
    bias: u64,
    lo: u64,
    hi: u64,
}

impl Fetch for Guest {
    fn word(&self, pc: u64) -> Option<u32> {
        if pc < self.lo || pc >= self.hi || pc & 3 != 0 {
            return None;
        }
        Some(unsafe { *(pc.wrapping_add(self.bias) as *const u32) })
    }
}

fn sysname(id: u16) -> String {
    let op0 = 2 | ((id >> 14) & 1);
    format!("S{}_{}_C{}_C{}_{}", op0, (id >> 11) & 7, (id >> 7) & 15, (id >> 3) & 15, id & 7)
}

fn apply_chain(img: &macho::Image, bias: u64) -> (usize, usize) {
    let base = img.lo();
    let mut count = 0;
    let mut bad = 0;
    for &start in &img.chain_starts {
        let mut va = img.va_of_fileoff(start).expect("chain start inside a segment");
        loop {
            let p = va.wrapping_add(bias) as *mut u64;
            let v = unsafe { *p };
            let next = (v >> 51) & 0xfff;
            let target = v & 0x3fff_ffff;
            let new = base + target;
            if new < img.lo() || new >= img.hi() {
                bad += 1;
            }
            unsafe { *p = new };
            count += 1;
            if next == 0 {
                break;
            }
            va += next * 4;
        }
    }
    (count, bad)
}

fn reg(ctx: &Ctx, r: u8) -> u64 {
    if r == 31 {
        0
    } else {
        ctx.x[r as usize]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| "sptm.bin".into());
    let limit_events: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(400);
    let data = std::fs::read(&path).expect("read image");
    let img = macho::parse(&data);
    let span = img.hi() - img.lo();
    let phys = !args.iter().any(|a| a == "--virt");
    let lo = if phys { PBASE } else { img.lo() };
    let hi = lo + span + SCRATCH;
    let len = ((hi - lo) as usize + 0xfff) & !0xfff;
    let host = if phys { PBASE } else { HOST_BASE };
    let bias = host.wrapping_sub(lo);
    let _region = Region::fixed(host as usize, len, false).expect("map guest window");
    unsafe {
        BIAS = bias;
    }
    for s in &img.segs {
        if s.fsize > 0 {
            let dst = (lo + (s.vm - img.lo())).wrapping_add(bias) as *mut u8;
            let n = s.fsize as usize;
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().add(s.off as usize), dst, n) };
        }
    }
    let entry = lo + (img.entry - img.lo());
    println!("image {} link lo {:#x} entry {:#x} chain format {} starts {:?}", path, img.lo(), img.entry, img.chain_format, img.chain_starts);
    if !phys {
        let (fixed, bad) = apply_chain(&img, bias);
        println!("chained fixups applied: {} (out of range: {})", fixed, bad);
    }
    install_segv();

    let args_va = lo + span + 0x1000;
    let stack_va = hi - 0x1000;
    unsafe {
        let a = args_va.wrapping_add(bias) as *mut u64;
        *a = 3;
        *a.add(1) = 0xfffffe0007004000;
        *a.add(2) = RAM_BASE;
        *a.add(3) = RAM_SIZE;
    }
    let mut cfg = Config::new(if phys { MemModel::Identity } else { MemModel::Biased });
    cfg.pc_base = lo;
    cfg.bias = bias;
    let mut runner = Runner::new(cfg);
    let mut ctx = Box::new(Ctx::default());
    ctx.pc = entry;
    ctx.x[0] = args_va;
    ctx.x[31] = stack_va;
    unsafe {
        CTX_PTR = &*ctx as *const Ctx;
    }
    let guest = Guest { bias, lo, hi };

    let mut sregs: HashMap<u16, u64> = HashMap::new();
    sregs.insert(sysreg_id(3, 0, 4, 2, 2), 0x4);
    sregs.insert(sysreg_id(3, 0, 0, 0, 5), 0x8000_0000);
    sregs.insert(sysreg_id(3, 6, 15, 12, 4), 1);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut events = 0usize;
    let mut blocks = 0usize;
    let mut last_pc = u64::MAX;
    let mut same = 0usize;
    let reason = loop {
        match runner.run_until_stop(&mut ctx, &guest, 4096) {
            Stop::Budget => {
                blocks += 4096;
                if blocks > 20_000_000 {
                    break "block budget".to_string();
                }
            }
            Stop::Refused(e) => break format!("refused at {:#x}: {}", ctx.pc, e),
            Stop::Interp(pc) => {
                let w = match guest.word(pc) {
                    Some(w) => w,
                    None => break format!("fetch outside image at {:#x}", pc),
                };
                if pc == last_pc {
                    same += 1;
                    if same > 50 {
                        break format!("spinning at {:#x}", pc);
                    }
                } else {
                    same = 0;
                    last_pc = pc;
                }
                events += 1;
                let say = events <= limit_events;
                match decode(w) {
                    Insn::Mrs { rt, id } => {
                        let v = sregs.get(&id).copied().unwrap_or(0);
                        if rt != 31 {
                            ctx.x[rt as usize] = v;
                        }
                        *counts.entry(format!("mrs {}", sysname(id))).or_default() += 1;
                        if say {
                            println!("#{} {:#x} mrs x{} <- {} = {:#x}", events, pc, rt, sysname(id), v);
                        }
                        ctx.pc = pc + 4;
                    }
                    Insn::Msr { rt, id } => {
                        let v = reg(&ctx, rt);
                        sregs.insert(id, v);
                        *counts.entry(format!("msr {}", sysname(id))).or_default() += 1;
                        if say {
                            println!("#{} {:#x} msr {} <- x{} = {:#x}", events, pc, sysname(id), rt, v);
                        }
                        ctx.pc = pc + 4;
                    }
                    Insn::MsrImm { field, imm } => {
                        *counts.entry(format!("msr-imm {}", field)).or_default() += 1;
                        if say {
                            println!("#{} {:#x} msr-imm field {} imm {}", events, pc, field, imm);
                        }
                        ctx.pc = pc + 4;
                    }
                    Insn::Sys { l, op1, crn, crm, op2, rt } => {
                        *counts.entry(format!("sys {} {} c{} c{} {}", if l { "sysl" } else { "sys" }, op1, crn, crm, op2)).or_default() += 1;
                        if say {
                            println!("#{} {:#x} sys op1 {} c{} c{} op2 {} x{} (ignored)", events, pc, op1, crn, crm, op2, rt);
                        }
                        ctx.pc = pc + 4;
                    }
                    Insn::Hint { op } if op == 2 || op == 3 => {
                        break format!("wait-for-event/interrupt at {:#x} (x0 {:#x})", pc, ctx.x[0]);
                    }
                    Insn::Hint { .. } => {
                        ctx.pc = pc + 4;
                    }
                    Insn::Exc { kind, imm } => {
                        let k = match kind {
                            ExcKind::Svc => "svc",
                            ExcKind::Hvc => "hvc",
                            ExcKind::Smc => "smc",
                            ExcKind::Brk => "brk",
                            ExcKind::Hlt => "hlt",
                            ExcKind::Udf => "udf",
                        };
                        break format!("{} #{:#x} at {:#x}", k, imm, pc);
                    }
                    Insn::Eret => break format!("eret at {:#x}", pc),
                    other => break format!("unhandled instruction {:08x} ({}) at {:#x}", w, other.name(), pc),
                }
            }
        }
    };
    println!("stopped: {}", reason);
    println!("after {} interpreter events, {} blocks", events, blocks);
    println!("pc {:#x}", ctx.pc);
    for r in 0..31 {
        print!("x{:<2} {:016x}  ", r, ctx.x[r]);
        if r % 4 == 3 {
            println!();
        }
    }
    println!("sp  {:016x}", ctx.x[31]);
    let mut top: Vec<_> = counts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    println!("system access summary:");
    for (k, v) in top.iter().take(40) {
        println!("  {:6} {}", v, k);
    }
}
