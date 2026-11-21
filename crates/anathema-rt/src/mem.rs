use anathema_xlat::run::Region;
use std::collections::HashMap;

pub const RAM_BASE: u64 = 0x7000_0000;
pub const RAM_SIZE: u64 = 0x1_0000_0000;

pub struct Ram {
    pub base: u64,
    pub len: u64,
    _region: Region,
}

impl Ram {
    pub fn new(base: u64, len: u64) -> Ram {
        let region = Region::fixed(base as usize, len as usize, false).expect("map guest ram");
        Ram { base, len, _region: region }
    }

    pub fn contains(&self, pa: u64, n: u64) -> bool {
        pa >= self.base && pa.wrapping_add(n) <= self.base + self.len && pa.wrapping_add(n) >= pa
    }

    pub fn read(&self, pa: u64, size: usize) -> u64 {
        let p = pa as *const u8;
        unsafe {
            match size {
                1 => *p as u64,
                2 => (p as *const u16).read_unaligned() as u64,
                4 => (p as *const u32).read_unaligned() as u64,
                _ => (p as *const u64).read_unaligned(),
            }
        }
    }

    pub fn write(&self, pa: u64, size: usize, v: u64) {
        let p = pa as *mut u8;
        unsafe {
            match size {
                1 => *p = v as u8,
                2 => (p as *mut u16).write_unaligned(v as u16),
                4 => (p as *mut u32).write_unaligned(v as u32),
                _ => (p as *mut u64).write_unaligned(v),
            }
        }
    }

    pub fn bytes(&self, pa: u64, len: usize) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(pa as *mut u8, len) }
    }
}

pub const FB_BASE: u64 = 0x6000_0000;
pub const FB_BYTES: usize = 1920 * 1080 * 4;

pub struct Bus {
    pub fb: Vec<u8>,
    pub uart: String,
    regs: HashMap<u64, u64>,
    pub log_budget: usize,
    pub seen: HashMap<u64, usize>,
}

impl Bus {
    pub fn new() -> Bus {
        Bus { fb: vec![0u8; FB_BYTES], uart: String::new(), regs: HashMap::new(), log_budget: 300, seen: HashMap::new() }
    }

    fn note(&mut self, kind: &str, pa: u64, size: usize, v: u64) {
        let region = pa & !0xffff;
        let n = self.seen.entry(region).or_insert(0);
        *n += 1;
        if self.log_budget > 0 && *n <= 40 {
            self.log_budget -= 1;
            println!("  mmio {} {:#x} size {} = {:#x}", kind, pa, size, v);
        }
    }

    pub fn read(&mut self, pa: u64, size: usize) -> u64 {
        if pa >= FB_BASE && pa + size as u64 <= FB_BASE + FB_BYTES as u64 {
            let o = (pa - FB_BASE) as usize;
            let mut b = [0u8; 8];
            b[..size].copy_from_slice(&self.fb[o..o + size]);
            return u64::from_le_bytes(b);
        }
        let v = match pa {
            0x2001_0018 => 0x90,
            0x2001_0fe0 => 0x11,
            0x2001_0fe4 => 0x10,
            0x2001_0fe8 => 0x34,
            0x2001_0fec => 0x00,
            0x2001_0ff0 => 0x0d,
            0x2001_0ff4 => 0xf0,
            0x2001_0ff8 => 0x05,
            0x2001_0ffc => 0xb1,
            _ => self.regs.get(&pa).copied().unwrap_or(0),
        };
        self.note("r", pa, size, v);
        v
    }

    pub fn write(&mut self, pa: u64, size: usize, v: u64) {
        if pa >= FB_BASE && pa + size as u64 <= FB_BASE + FB_BYTES as u64 {
            let o = (pa - FB_BASE) as usize;
            self.fb[o..o + size].copy_from_slice(&v.to_le_bytes()[..size]);
            return;
        }
        if pa == 0x2001_0000 {
            let c = (v & 0xff) as u8 as char;
            self.uart.push(c);
            print!("{}", c);
            return;
        }
        self.regs.insert(pa, v);
        self.note("w", pa, size, v);
    }
}
