use core::ptr::{read_volatile, write_volatile};

#[derive(Clone, Copy)]
pub struct Bdf {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
}

impl Bdf {
    pub fn from_id(id: u16) -> Bdf {
        Bdf { bus: (id >> 8) as u8, dev: ((id >> 3) & 0x1f) as u8, func: (id & 7) as u8 }
    }
}

fn address(ecam: u64, at: Bdf, offset: u64) -> u64 {
    ecam + ((at.bus as u64) << 20) + ((at.dev as u64) << 15) + ((at.func as u64) << 12) + offset
}

pub fn read32(ecam: u64, at: Bdf, offset: u64) -> u32 {
    unsafe { read_volatile(address(ecam, at, offset) as *const u32) }
}

pub fn write32(ecam: u64, at: Bdf, offset: u64, value: u32) {
    unsafe { write_volatile(address(ecam, at, offset) as *mut u32, value) }
}

pub fn find_cap(ecam: u64, at: Bdf, id: u8) -> Option<u64> {
    if read32(ecam, at, 4) >> 16 & 0x10 == 0 {
        return None;
    }
    let mut pointer = (read32(ecam, at, 0x34) & 0xfc) as u64;
    let mut guard = 0;
    while pointer >= 0x40 && guard < 48 {
        let head = read32(ecam, at, pointer);
        if head & 0xff == id as u32 {
            return Some(pointer);
        }
        pointer = ((head >> 8) & 0xfc) as u64;
        guard += 1;
    }
    None
}
