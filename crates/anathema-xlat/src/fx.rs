use core::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    h: u64,
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.h = (self.h.rotate_left(5) ^ *b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.h = (self.h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_u16(&mut self, v: u16) {
        self.write_u64(v as u64);
    }

    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }

    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }

    fn finish(&self) -> u64 {
        self.h
    }
}

pub type HashMap<K, V> = hashbrown::HashMap<K, V, BuildHasherDefault<FxHasher>>;
