mod boot;
mod dt;
mod machine;
mod macho;
mod mem;
mod mmu;
mod simd;

use anathema_xlat::run::{Ctx, Runner, Stop};
use anathema_xlat::trans::{Config, MemModel};
use machine::{Fetcher, Machine, Step};
use mem::{Ram, RAM_BASE, RAM_SIZE};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

static mut CTX_PTR: *const Ctx = std::ptr::null();

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
        eprintln!("host fault at {:#x} in block starting at guest pc {:#x}", addr, pc);
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("dt") {
        let d = std::fs::read(&args[2]).expect("read devicetree");
        let root = dt::parse(&d);
        let depth: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
        let mut node = &root;
        if let Some(path) = args.get(4) {
            for part in path.split('/') {
                node = node.child(part).expect("no such node");
            }
        }
        dt::dump(node, 0, depth, &[]);
        return;
    }
    let kpath = args.get(1).cloned().unwrap_or_else(|| "kc-vm.macho".into());
    let dpath = args.get(2).cloned().unwrap_or_else(|| "dt-vm.bin".into());
    let trace: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let cmdline = std::env::var("RT_BOOTARGS").unwrap_or_else(|_| "-v serial=3 debug=0x14e".into());
    let kdata = std::fs::read(&kpath).expect("read kernel");
    let img = macho::parse(&kdata);
    let dtb = std::fs::read(&dpath).expect("read devicetree");
    let ram = Ram::new(RAM_BASE, RAM_SIZE);
    let loaded = boot::load_kernel(&ram, &img, &kdata, &dtb, &cmdline);
    let mut m = Machine::new(ram);
    m.trace_budget = trace;
    install_segv();
    let mut ctx = Box::new(Ctx::default());
    ctx.pc = loaded.entry_pa;
    ctx.x[0] = loaded.args_pa;
    unsafe {
        CTX_PTR = &*ctx as *const Ctx;
    }
    println!("starting at physical {:#x}, boot args {:#x}", ctx.pc, ctx.x[0]);

    let mut runners: HashMap<u64, Runner> = HashMap::new();
    let mut blocks = 0u64;
    let mut last = (u64::MAX, 0usize);
    let limit: u64 = std::env::var("RT_BLOCKS").ok().and_then(|s| s.parse().ok()).unwrap_or(400_000_000);
    let ring_on = std::env::var("RT_RING").is_ok();
    let trace_on = ring_on || std::env::var("RT_TRACE").is_ok();
    let mut ring: VecDeque<u64> = VecDeque::new();
    let reason = loop {
        let key = ctx.pc >> 30;
        let runner = runners.entry(key).or_insert_with(|| {
            let mut cfg = Config::new(MemModel::Exit);
            cfg.pc_base = key << 30;
            if ring_on {
                cfg.max_insns = 1;
            }
            Runner::new(cfg)
        });
        if trace_on {
            ring.push_back(ctx.pc);
            if ring.len() > 48 {
                ring.pop_front();
            }
        }
        let stop = {
            let f = Fetcher { m: RefCell::new(&mut m) };
            runner.run_until_stop(&mut ctx, &f, if ring_on { 1 } else { 4096 })
        };
        match stop {
            Stop::Budget => {
                blocks += if ring_on { 1 } else { 4096 };
                if blocks > limit {
                    break format!("block budget reached at pc {:#x}", ctx.pc);
                }
            }
            Stop::Refused(e) => {
                let mut flush = false;
                match m.step(&mut ctx, &mut flush) {
                    Step::Next => {}
                    Step::Stop(r) => break format!("{} (translator: {})", r, e),
                }
            }
            Stop::Interp(pc) => {
                if pc == last.0 {
                    last.1 += 1;
                    if last.1 > 200_000 {
                        break format!("spinning at {:#x}", pc);
                    }
                } else {
                    last = (pc, 0);
                }
                let mut flush = false;
                match m.step(&mut ctx, &mut flush) {
                    Step::Next => {}
                    Step::Stop(r) => break r,
                }
                if flush {
                    for r in runners.values_mut() {
                        r.reset_cache();
                    }
                }
            }
        }
    };
    println!();
    if trace_on {
        println!("last program counters:");
        for pc in &ring {
            let w = m.fetch_word(*pc).unwrap_or(0);
            println!("  {:#x}  {:08x}", pc, w);
        }
    }
    println!("stopped: {}", reason);
    println!("interpreter steps {}, mmu {}", m.events, if m.mmu.enabled() { "on" } else { "off" });
    println!("pc {:#x}", ctx.pc);
    for r in 0..31 {
        print!("x{:<2} {:016x}  ", r, ctx.x[r]);
        if r % 4 == 3 {
            println!();
        }
    }
    println!("sp  {:016x}", ctx.x[31]);
    println!("sctlr {:#x} tcr {:#x} ttbr0 {:#x} ttbr1 {:#x}", m.mmu.sctlr, m.mmu.tcr, m.mmu.ttbr0, m.mmu.ttbr1);
    match m.translate(ctx.pc) {
        Ok(pa) => {
            println!("pc translates to {:#x}", pa);
            if m.ram.contains(pa, 16) {
                println!("bytes there: {:08x} {:08x} {:08x} {:08x}", m.ram.read(pa, 4), m.ram.read(pa + 4, 4), m.ram.read(pa + 8, 4), m.ram.read(pa + 12, 4));
            }
        }
        Err(e) => println!("pc does not translate: {}", e),
    }
    let expect = boot::KERNEL_PA + (ctx.pc.wrapping_sub(loaded.virt_base));
    if m.ram.contains(expect, 16) {
        println!("linear expectation {:#x}: {:08x} {:08x} {:08x} {:08x}", expect, m.ram.read(expect, 4), m.ram.read(expect + 4, 4), m.ram.read(expect + 8, 4), m.ram.read(expect + 12, 4));
    }
    let mut top: Vec<_> = m.counts.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1));
    println!("system access summary:");
    for (k, v) in top.iter().take(30) {
        println!("  {:8} {}", v, k);
    }
    if !m.bus.uart.is_empty() {
        println!("uart output ({} bytes) shown above", m.bus.uart.len());
    }
}
