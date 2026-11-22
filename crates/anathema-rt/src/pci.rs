use crate::gic::{Gic, MSI_BASE};
use crate::mem::Ram;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

pub const ECAM_BASE: u64 = 0x4000_0000;
pub const ECAM_SIZE: u64 = 0x1000_0000;
const BAR_SIZE: u64 = 0x4000;
const QSIZE: u16 = 256;

pub trait Disk {
    fn sectors(&self) -> u64;
    fn read(&mut self, sector: u64, out: &mut [u8]) -> bool;
}

pub struct Overlay {
    base: Box<dyn Disk>,
    writes: BTreeMap<u64, [u8; 512]>,
}

impl Overlay {
    pub fn new(base: Box<dyn Disk>) -> Overlay {
        Overlay { base, writes: BTreeMap::new() }
    }

    pub fn sectors(&self) -> u64 {
        self.base.sectors()
    }

    pub fn read(&mut self, sector: u64, out: &mut [u8]) -> bool {
        for (i, chunk) in out.chunks_mut(512).enumerate() {
            let s = sector + i as u64;
            if let Some(w) = self.writes.get(&s) {
                chunk.copy_from_slice(&w[..chunk.len()]);
            } else if !self.base.read(s, chunk) {
                return false;
            }
        }
        true
    }

    pub fn write(&mut self, sector: u64, data: &[u8]) {
        for (i, chunk) in data.chunks(512).enumerate() {
            let mut b = [0u8; 512];
            b[..chunk.len()].copy_from_slice(chunk);
            self.writes.insert(sector + i as u64, b);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Queue {
    size: u16,
    enable: u16,
    vector: u16,
    desc: u64,
    avail: u64,
    used: u64,
    last_avail: u16,
}

pub struct Pci {
    pub disk: Option<Overlay>,
    cmd: u16,
    bar0: u32,
    bar1: u32,
    int_line: u8,
    msix_ctl: u16,
    dev_feat_sel: u32,
    drv_feat_sel: u32,
    drv_feat: [u32; 2],
    status: u8,
    queue_sel: u16,
    msix_config: u16,
    q: Queue,
    isr: u8,
    table: [u32; 64],
    pba: u64,
    pub log: usize,
    pub cfg_log: usize,
    pub reqs: u64,
}

fn cap_bytes() -> [u8; 256] {
    let mut c = [0u8; 256];
    let put16 = |c: &mut [u8; 256], o: usize, v: u16| c[o..o + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |c: &mut [u8; 256], o: usize, v: u32| c[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put16(&mut c, 0x00, 0x1af4);
    put16(&mut c, 0x02, 0x1042);
    put16(&mut c, 0x06, 0x0010);
    c[0x08] = 1;
    c[0x0b] = 0x01;
    c[0x0a] = 0x80;
    c[0x0e] = 0;
    put16(&mut c, 0x2c, 0x1af4);
    put16(&mut c, 0x2e, 0x0002);
    c[0x34] = 0x40;
    c[0x3d] = 1;
    let vcap = |c: &mut [u8; 256], at: usize, next: usize, len: usize, ty: u8, off: u32, l: u32| {
        c[at] = 9;
        c[at + 1] = next as u8;
        c[at + 2] = len as u8;
        c[at + 3] = ty;
        c[at + 4] = 0;
        c[at + 8..at + 12].copy_from_slice(&off.to_le_bytes());
        c[at + 12..at + 16].copy_from_slice(&l.to_le_bytes());
    };
    vcap(&mut c, 0x40, 0x50, 16, 1, 0x0000, 0x38);
    vcap(&mut c, 0x50, 0x64, 20, 2, 0x1000, 0x1000);
    put32(&mut c, 0x60, 4);
    vcap(&mut c, 0x64, 0x74, 16, 3, 0x2000, 4);
    vcap(&mut c, 0x74, 0x84, 16, 4, 0x3000, 0x40);
    c[0x84] = 0x11;
    c[0x85] = 0;
    put16(&mut c, 0x86, 7);
    put32(&mut c, 0x88, 0x3800);
    put32(&mut c, 0x8c, 0x3f00);
    c
}

impl Pci {
    pub fn new() -> Pci {
        Pci {
            disk: None,
            cmd: 0,
            bar0: 4,
            bar1: 0,
            int_line: 0,
            msix_ctl: 7,
            dev_feat_sel: 0,
            drv_feat_sel: 0,
            drv_feat: [0; 2],
            status: 0,
            queue_sel: 0,
            msix_config: 0xffff,
            q: Queue { size: QSIZE, vector: 0xffff, ..Default::default() },
            isr: 0,
            table: [0; 64],
            pba: 0,
            log: 3000,
            cfg_log: 100,
            reqs: 0,
        }
    }

    pub fn claims(&self, pa: u64) -> bool {
        if pa >= ECAM_BASE && pa < ECAM_BASE + ECAM_SIZE {
            return true;
        }
        let b = self.bar_addr();
        self.cmd & 2 != 0 && b != 0 && pa >= b && pa < b + BAR_SIZE
    }

    pub fn bar_base(&self) -> u64 {
        self.bar_addr()
    }

    fn bar_addr(&self) -> u64 {
        ((self.bar1 as u64) << 32) | (self.bar0 as u64 & !0xf)
    }

    fn cfg_read(&self, off: usize) -> u32 {
        let c = cap_bytes();
        let w = |o: usize| u32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]]);
        match off & !3 {
            0x04 => self.cmd as u32 | (0x0010 << 16),
            0x10 => (self.bar0 & !((BAR_SIZE as u32) - 1) & !0xf) | 4,
            0x14 => self.bar1,
            0x3c => (w(0x3c) & !0xff) | self.int_line as u32,
            0x84 => 0x11 | (0 << 8) | ((self.msix_ctl as u32) << 16),
            o if o < 0x100 => w(o),
            _ => 0,
        }
    }

    pub fn ecam_read(&mut self, off: u64, size: usize) -> u64 {
        let dev = (off >> 15) & 0x1f;
        let func = (off >> 12) & 7;
        let bus = off >> 20;
        let reg = (off & 0xfff) as usize;
        let ones = if size >= 8 { u64::MAX } else { (1u64 << (size * 8)) - 1 };
        if bus != 0 || dev != 1 || func != 0 || reg >= 0x100 {
            return ones;
        }
        let v = self.cfg_read(reg) as u64 >> ((reg & 3) * 8);
        let r = v & ones;
        if self.cfg_log > 0 {
            self.cfg_log -= 1;
            println!("  pci cfg r {:#x} size {} = {:#x}", reg, size, r);
        }
        r
    }

    pub fn ecam_write(&mut self, off: u64, size: usize, v: u64) {
        let dev = (off >> 15) & 0x1f;
        let func = (off >> 12) & 7;
        let bus = off >> 20;
        let reg = (off & 0xfff) as usize;
        if bus != 0 || dev != 1 || func != 0 || reg >= 0x100 {
            return;
        }
        if self.cfg_log > 0 {
            self.cfg_log -= 1;
            println!("  pci cfg w {:#x} size {} = {:#x}", reg, size, v);
        }
        let cur = self.cfg_read(reg) as u64;
        let sh = (reg & 3) * 8;
        let mask = (if size >= 4 { 0xffff_ffffu64 } else { (1u64 << (size * 8)) - 1 }) << sh;
        let nv = ((cur & !mask) | ((v << sh) & mask)) as u32;
        match reg & !3 {
            0x04 => self.cmd = (nv & 0x0407) as u16,
            0x10 => self.bar0 = nv,
            0x14 => self.bar1 = nv,
            0x3c => self.int_line = nv as u8,
            0x84 => self.msix_ctl = (self.msix_ctl & 0x3fff) | ((nv >> 16) as u16 & 0xc000),
            _ => {}
        }
    }

    fn blk_cfg(&self) -> [u8; 0x40] {
        let mut c = [0u8; 0x40];
        let sectors = self.disk.as_ref().map(|d| d.sectors()).unwrap_or(0);
        c[0..8].copy_from_slice(&sectors.to_le_bytes());
        c[8..12].copy_from_slice(&0x20000u32.to_le_bytes());
        c[12..16].copy_from_slice(&126u32.to_le_bytes());
        c[20..24].copy_from_slice(&512u32.to_le_bytes());
        c
    }

    pub fn bar_read(&mut self, off: u64, size: usize) -> u64 {
        let off = off as usize;
        let ones = if size >= 8 { u64::MAX } else { (1u64 << (size * 8)) - 1 };
        let v: u64 = match off {
            0x0000..=0x0037 => match off {
                0x00 => self.dev_feat_sel as u64,
                0x04 => {
                    let f: u64 = (1 << 32) | (1 << 6) | (1 << 9);
                    ((f >> (32 * (self.dev_feat_sel & 1) as u64)) & 0xffff_ffff) as u64
                }
                0x08 => self.drv_feat_sel as u64,
                0x0c => self.drv_feat[(self.drv_feat_sel & 1) as usize] as u64,
                0x10 => self.msix_config as u64,
                0x12 => 1,
                0x14 => self.status as u64,
                0x15 => 0,
                0x16 => self.queue_sel as u64,
                0x18 => self.q.size as u64,
                0x1a => self.q.vector as u64,
                0x1c => self.q.enable as u64,
                0x1e => 0,
                0x20 => self.q.desc & 0xffff_ffff,
                0x24 => self.q.desc >> 32,
                0x28 => self.q.avail & 0xffff_ffff,
                0x2c => self.q.avail >> 32,
                0x30 => self.q.used & 0xffff_ffff,
                0x34 => self.q.used >> 32,
                _ => 0,
            },
            0x2000..=0x2003 => {
                let r = self.isr as u64;
                self.isr = 0;
                r
            }
            0x3000..=0x303f => {
                let c = self.blk_cfg();
                let mut b = [0u8; 8];
                for i in 0..size.min(8) {
                    b[i] = *c.get(off - 0x3000 + i).unwrap_or(&0);
                }
                u64::from_le_bytes(b)
            }
            0x3800..=0x39ff => {
                let i = (off - 0x3800) / 4;
                (*self.table.get(i).unwrap_or(&0) as u64) >> (((off & 3) * 8) as u64)
            }
            0x3f00..=0x3f07 => self.pba >> ((off - 0x3f00) * 8),
            _ => 0,
        };
        let r = v & ones;
        if self.log > 0 && !(0x3000..0x3040).contains(&off) || self.log > 0 {
            self.log -= 1;
            println!("  virtio r {:#x} size {} = {:#x}", off, size, r);
        }
        r
    }

    pub fn bar_write(&mut self, off: u64, size: usize, v: u64, ram: &Ram, gic: &mut Gic) {
        let off = off as usize;
        if self.log > 0 {
            self.log -= 1;
            println!("  virtio w {:#x} size {} = {:#x}", off, size, v);
        }
        match off {
            0x00 => self.dev_feat_sel = v as u32,
            0x08 => self.drv_feat_sel = v as u32,
            0x0c => self.drv_feat[(self.drv_feat_sel & 1) as usize] = v as u32,
            0x10 => self.msix_config = v as u16,
            0x14 => {
                self.status = v as u8;
                if v == 0 {
                    self.q = Queue { size: QSIZE, vector: 0xffff, ..Default::default() };
                    self.isr = 0;
                }
            }
            0x16 => self.queue_sel = v as u16,
            0x18 => {
                if v != 0 && v <= QSIZE as u64 {
                    self.q.size = v as u16
                }
            }
            0x1a => self.q.vector = v as u16,
            0x1c => self.q.enable = v as u16,
            0x20 => self.q.desc = (self.q.desc & !0xffff_ffff) | (v & 0xffff_ffff),
            0x24 => self.q.desc = (self.q.desc & 0xffff_ffff) | (v << 32),
            0x28 => self.q.avail = (self.q.avail & !0xffff_ffff) | (v & 0xffff_ffff),
            0x2c => self.q.avail = (self.q.avail & 0xffff_ffff) | (v << 32),
            0x30 => self.q.used = (self.q.used & !0xffff_ffff) | (v & 0xffff_ffff),
            0x34 => self.q.used = (self.q.used & 0xffff_ffff) | (v << 32),
            0x1000..=0x1fff => self.process(ram, gic),
            0x3800..=0x39ff => {
                let i = (off - 0x3800) / 4;
                if i < self.table.len() {
                    self.table[i] = v as u32;
                }
            }
            _ => {}
        }
    }

    fn raise(&mut self, gic: &mut Gic) {
        let vec = self.q.vector;
        if self.msix_ctl & 0x8000 != 0 && vec != 0xffff && (vec as usize) < 8 {
            let t = vec as usize * 4;
            let addr = self.table[t] as u64 | ((self.table[t + 1] as u64) << 32);
            let data = self.table[t + 2] as u64;
            let ctl = self.table[t + 3];
            if ctl & 1 != 0 || self.msix_ctl & 0x4000 != 0 {
                self.pba |= 1 << vec;
            } else if addr >= MSI_BASE && addr < MSI_BASE + 0x1_0000 {
                gic.msi((addr - MSI_BASE) as usize, Some(data));
            }
        } else {
            self.isr |= 1;
        }
    }

    fn process(&mut self, ram: &Ram, gic: &mut Gic) {
        if self.q.enable == 0 || self.disk.is_none() {
            return;
        }
        let size = self.q.size as u64;
        let mut did = false;
        loop {
            let avail_idx = ram.read(self.q.avail + 2, 2) as u16;
            if self.q.last_avail == avail_idx {
                break;
            }
            let slot = (self.q.last_avail as u64) % size;
            let head = ram.read(self.q.avail + 4 + slot * 2, 2) as u16;
            self.q.last_avail = self.q.last_avail.wrapping_add(1);
            let mut chain: Vec<(u64, u32, bool)> = Vec::new();
            let mut idx = head as u64;
            for _ in 0..64 {
                let d = self.q.desc + idx * 16;
                let addr = ram.read(d, 8);
                let len = ram.read(d + 8, 4) as u32;
                let flags = ram.read(d + 12, 2);
                let next = ram.read(d + 14, 2);
                chain.push((addr, len, flags & 2 != 0));
                if flags & 1 == 0 {
                    break;
                }
                idx = next;
            }
            let mut total = 0u32;
            let mut status = 0u8;
            if chain.len() >= 2 && ram.contains(chain[0].0, 16) {
                let ty = ram.read(chain[0].0, 4) as u32;
                let sector = ram.read(chain[0].0 + 8, 8);
                self.reqs += 1;
                if self.log > 0 && self.reqs <= 400 {
                    println!("  virtio blk req type {} sector {} segs {}", ty, sector, chain.len() - 2);
                }
                let data = &chain[1..chain.len() - 1];
                let disk = self.disk.as_mut().unwrap();
                match ty {
                    0 => {
                        let mut s = sector;
                        for (addr, len, _) in data {
                            let mut buf = vec![0u8; *len as usize];
                            if !disk.read(s, &mut buf) {
                                status = 1;
                                break;
                            }
                            if ram.contains(*addr, *len as u64) {
                                ram.bytes(*addr, *len as usize).copy_from_slice(&buf);
                            }
                            s += (*len as u64 + 511) / 512;
                            total += *len;
                        }
                    }
                    1 => {
                        let mut s = sector;
                        for (addr, len, _) in data {
                            if ram.contains(*addr, *len as u64) {
                                let buf = ram.bytes(*addr, *len as usize).to_vec();
                                disk.write(s, &buf);
                            }
                            s += (*len as u64 + 511) / 512;
                        }
                    }
                    4 => {}
                    8 => {
                        for (addr, len, _) in data {
                            if ram.contains(*addr, *len as u64) {
                                let b = ram.bytes(*addr, *len as usize);
                                let id = b"anathema-root";
                                for (i, x) in b.iter_mut().enumerate() {
                                    *x = *id.get(i).unwrap_or(&0);
                                }
                            }
                        }
                    }
                    _ => status = 2,
                }
                let (saddr, _, _) = chain[chain.len() - 1];
                if ram.contains(saddr, 1) {
                    ram.write(saddr, 1, status as u64);
                }
                total += 1;
            }
            let used_idx = ram.read(self.q.used + 2, 2) as u16;
            let us = (used_idx as u64) % size;
            ram.write(self.q.used + 4 + us * 8, 4, head as u64);
            ram.write(self.q.used + 8 + us * 8, 4, total as u64);
            ram.write(self.q.used + 2, 2, used_idx.wrapping_add(1) as u64);
            did = true;
        }
        if did {
            self.raise(gic);
        }
    }
}
