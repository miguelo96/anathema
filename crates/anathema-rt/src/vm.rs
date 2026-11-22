use crate::boot;
use crate::machine::{Fetcher, Machine, Step};
use crate::macho;
use crate::mem::Ram;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use anathema_xlat::fx::HashMap;
use anathema_xlat::run::{Ctx, Runner, Stop};
use anathema_xlat::trans::{Config, MemModel};
use core::cell::RefCell;

pub struct Vm {
    pub m: Machine,
    pub ctx: Box<Ctx>,
    runners: HashMap<u64, Runner>,
    last: (u64, usize),
}

impl Vm {
    pub fn new(ram: Ram, kdata: &[u8], dtb: &[u8], cmdline: &str, ramdisk: Option<&[u8]>) -> Vm {
        let img = macho::parse(kdata);
        let loaded = boot::load_kernel(&ram, &img, kdata, dtb, cmdline, ramdisk);
        let m = Machine::new(ram);
        let mut ctx = Box::new(Ctx::default());
        ctx.pc = loaded.entry_pa;
        ctx.x[0] = loaded.args_pa;
        Vm { m, ctx, runners: HashMap::default(), last: (u64::MAX, 0) }
    }

    pub fn run(&mut self, slices: u32) -> Option<String> {
        let mut n = 0;
        loop {
            if n >= slices {
                return None;
            }
            n += 1;
            let key = self.ctx.pc >> 30;
            let runner = self.runners.entry(key).or_insert_with(|| {
                let mut cfg = Config::new(MemModel::Exit);
                cfg.indirect_flags_dead = false;
                cfg.pc_base = key << 30;
                Runner::new(cfg)
            });
            let stop = {
                let f = Fetcher { m: RefCell::new(&mut self.m) };
                runner.run_until_stop(&mut self.ctx, &f, 4096)
            };
            match stop {
                Stop::Budget => {}
                Stop::Refused(e) => {
                    let mut flush = false;
                    match self.m.step(&mut self.ctx, &mut flush) {
                        Step::Next => {}
                        Step::Stop(r) => return Some(format!("{} (translator: {})", r, e)),
                    }
                }
                Stop::Interp(pc) => {
                    if pc == self.last.0 {
                        self.last.1 += 1;
                        if self.last.1 > 200_000 {
                            return Some(format!("spinning at {:#x}", pc));
                        }
                    } else {
                        self.last = (pc, 0);
                    }
                    let mut flush = false;
                    match self.m.step(&mut self.ctx, &mut flush) {
                        Step::Next => {}
                        Step::Stop(r) => return Some(r),
                    }
                    if flush {
                        for (k, r) in self.runners.iter_mut() {
                            if *k < (1u64 << 33) {
                                r.reset_cache();
                            }
                        }
                    }
                }
            }
        }
    }
}
