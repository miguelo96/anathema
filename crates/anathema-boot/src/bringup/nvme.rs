use super::apic::rdtsc;
use super::pcicfg::{self, Bdf};
use super::Prep;
use crate::blog;
use core::ptr::{read_volatile, write_volatile};

const QUEUE: usize = 16;
const ADMIN_DELETE_SQ: u8 = 0x00;
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_DELETE_CQ: u8 = 0x04;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const IO_READ: u8 = 0x02;

pub const DMA_PAGES: usize = 16;

pub struct Drive {
    pub model: [u8; 40],
    pub firmware: [u8; 8],
    pub namespaces: u32,
    pub nsze: u64,
    pub lba_shift: u32,
    pub version: u32,
}

pub struct Nvme {
    bar: u64,
    stride: u64,
    tsc_hz: u64,
    admin_sq: u64,
    admin_cq: u64,
    io_sq: u64,
    io_cq: u64,
    pub scratch: u64,
    pub data: u64,
    admin_tail: usize,
    admin_head: usize,
    admin_phase: u32,
    io_tail: usize,
    io_head: usize,
    io_phase: u32,
    cid: u16,
    pub lba_shift: u32,
}

fn r32(bar: u64, offset: u64) -> u32 {
    unsafe { read_volatile((bar + offset) as *const u32) }
}

fn w32(bar: u64, offset: u64, value: u32) {
    unsafe { write_volatile((bar + offset) as *mut u32, value) }
}

fn r64(bar: u64, offset: u64) -> u64 {
    let low = r32(bar, offset) as u64;
    let high = r32(bar, offset + 4) as u64;
    (high << 32) | low
}

fn w64(bar: u64, offset: u64, value: u64) {
    w32(bar, offset, value as u32);
    w32(bar, offset + 4, (value >> 32) as u32);
}

impl Nvme {
    fn wait_ready(&self, want: u32, ms: u64) -> bool {
        let begin = rdtsc();
        let limit = self.tsc_hz / 1000 * ms;
        while rdtsc().wrapping_sub(begin) < limit {
            if r32(self.bar, 0x1c) & 1 == want {
                return true;
            }
            core::hint::spin_loop();
        }
        r32(self.bar, 0x1c) & 1 == want
    }

    fn submit(&mut self, io: bool, sqe: &[u32; 16]) -> Result<(u32, u32), &'static str> {
        let opcode = (sqe[0] & 0xff) as u8;
        if io {
            if opcode != IO_READ {
                return Err("refused: only reads are allowed on the io queue");
            }
        } else if !matches!(opcode, ADMIN_DELETE_SQ | ADMIN_CREATE_SQ | ADMIN_DELETE_CQ | ADMIN_CREATE_CQ | ADMIN_IDENTIFY) {
            return Err("refused: admin opcode not on the allow list");
        }
        self.cid = self.cid.wrapping_add(1);
        let cid = self.cid;
        let (sq, cq, tail, head, phase, queue) = if io {
            (self.io_sq, self.io_cq, &mut self.io_tail, &mut self.io_head, &mut self.io_phase, 1u64)
        } else {
            (self.admin_sq, self.admin_cq, &mut self.admin_tail, &mut self.admin_head, &mut self.admin_phase, 0u64)
        };
        unsafe {
            let slot = (sq + (*tail * 64) as u64) as *mut u32;
            for (i, word) in sqe.iter().enumerate() {
                write_volatile(slot.add(i), if i == 0 { (*word & 0xffff) | ((cid as u32) << 16) } else { *word });
            }
            core::arch::asm!("mfence", options(nomem, nostack));
        }
        *tail = (*tail + 1) % QUEUE;
        let tail_value = *tail as u32;
        let bell = 0x1000 + (2 * queue) * self.stride;
        w32(self.bar, bell, tail_value);
        let begin = rdtsc();
        let limit = self.tsc_hz * 3;
        loop {
            let entry = (cq + (*head * 16) as u64) as *const u32;
            let dw3 = unsafe { read_volatile(entry.add(3)) };
            if (dw3 >> 16) & 1 == *phase {
                let dw0 = unsafe { read_volatile(entry) };
                *head += 1;
                if *head == QUEUE {
                    *head = 0;
                    *phase ^= 1;
                }
                let bell = 0x1000 + (2 * queue + 1) * self.stride;
                w32(self.bar, bell, *head as u32);
                if dw3 & 0xffff != cid as u32 {
                    return Err("completion id mismatch");
                }
                return Ok((dw0, dw3 >> 17));
            }
            if rdtsc().wrapping_sub(begin) > limit {
                return Err("command timed out");
            }
            core::hint::spin_loop();
        }
    }

    fn admin(&mut self, opcode: u8, nsid: u32, prp1: u64, cdw10: u32, cdw11: u32) -> Result<u32, &'static str> {
        let mut sqe = [0u32; 16];
        sqe[0] = opcode as u32;
        sqe[1] = nsid;
        sqe[6] = prp1 as u32;
        sqe[7] = (prp1 >> 32) as u32;
        sqe[10] = cdw10;
        sqe[11] = cdw11;
        let (dw0, status) = self.submit(false, &sqe)?;
        if status != 0 {
            blog!("nvme admin opcode {:#x} failed with status {:#x}", opcode, status);
            return Err("admin command failed");
        }
        Ok(dw0)
    }

    pub fn read(&mut self, lba: u64, sectors: u32) -> Result<(), &'static str> {
        let bytes = (sectors as u64) << self.lba_shift;
        if sectors == 0 || bytes > 8192 {
            return Err("read size not supported");
        }
        let mut sqe = [0u32; 16];
        sqe[0] = IO_READ as u32;
        sqe[1] = 1;
        sqe[6] = self.data as u32;
        sqe[7] = (self.data >> 32) as u32;
        if bytes > 4096 {
            let second = self.data + 4096;
            sqe[8] = second as u32;
            sqe[9] = (second >> 32) as u32;
        }
        sqe[10] = lba as u32;
        sqe[11] = (lba >> 32) as u32;
        sqe[12] = sectors - 1;
        let (_, status) = self.submit(true, &sqe)?;
        if status != 0 {
            blog!("nvme read of lba {} failed with status {:#x}", lba, status);
            return Err("read failed");
        }
        Ok(())
    }

    pub fn shutdown(&mut self) {
        let _ = self.admin(ADMIN_DELETE_SQ, 0, 0, 1, 0);
        let _ = self.admin(ADMIN_DELETE_CQ, 0, 0, 1, 0);
        let cc = r32(self.bar, 0x14);
        w32(self.bar, 0x14, (cc & !(3 << 14)) | (1 << 14));
        let begin = rdtsc();
        while rdtsc().wrapping_sub(begin) < self.tsc_hz && (r32(self.bar, 0x1c) >> 2) & 3 != 2 {
            core::hint::spin_loop();
        }
        w32(self.bar, 0x14, 0);
        self.wait_ready(0, 2000);
    }
}

pub fn bring_up(prep: &Prep, drive: &mut Drive) -> Option<Nvme> {
    if prep.nvme_bdf == 0xffff || prep.dma == 0 {
        blog!("nvme: no controller found");
        return None;
    }
    let at = Bdf::from_id(prep.nvme_bdf);
    let ecam = prep.ecam;
    let command = pcicfg::read32(ecam, at, 4);
    let bar_low = pcicfg::read32(ecam, at, 0x10);
    let bar_high = pcicfg::read32(ecam, at, 0x14);
    let bar = if (bar_low >> 1) & 3 == 2 { ((bar_high as u64) << 32) | (bar_low & !0xf) as u64 } else { (bar_low & !0xf) as u64 };
    blog!("nvme at {:02x}:{:02x}.{}: command {:#06x}, bar0 {:#x}", at.bus, at.dev, at.func, command & 0xffff, bar);
    if bar == 0 || bar_low & 1 == 1 || bar >= 2048 << 30 {
        blog!("nvme: bar0 unusable");
        return None;
    }
    if command & 0x6 != 0x6 {
        pcicfg::write32(ecam, at, 4, (command & 0xffff) | 0x6);
        blog!("nvme: enabled memory space and bus master");
    }
    let cap = r64(bar, 0);
    if cap == u64::MAX {
        blog!("nvme: registers read all ones");
        return None;
    }
    let version = r32(bar, 8);
    let mqes = (cap & 0xffff) as u32 + 1;
    let timeout = ((cap >> 24) & 0xff) * 500;
    let stride = 4u64 << ((cap >> 32) & 0xf);
    let css = (cap >> 37) & 0xff;
    let mpsmin = (cap >> 48) & 0xf;
    blog!("nvme: version {}.{}.{}, max queue entries {}, timeout {} ms, doorbell stride {}, css {:#x}, mps min {}", version >> 16, (version >> 8) & 0xff, version & 0xff, mqes, timeout, stride, css, mpsmin);
    if mpsmin != 0 || css & 1 == 0 || (mqes as usize) < QUEUE {
        blog!("nvme: controller capabilities not supported by this driver");
        return None;
    }
    drive.version = version;
    let base = prep.dma;
    unsafe { core::ptr::write_bytes(base as *mut u8, 0, DMA_PAGES * 4096) };
    let mut nvme = Nvme {
        bar,
        stride,
        tsc_hz: prep.tsc_hz,
        admin_sq: base,
        admin_cq: base + 4096,
        io_sq: base + 2 * 4096,
        io_cq: base + 3 * 4096,
        scratch: base + 4 * 4096,
        data: base + 5 * 4096,
        admin_tail: 0,
        admin_head: 0,
        admin_phase: 1,
        io_tail: 0,
        io_head: 0,
        io_phase: 1,
        cid: 0,
        lba_shift: 9,
    };
    let cc = r32(bar, 0x14);
    if cc & 1 == 1 {
        w32(bar, 0x14, cc & !1);
    }
    if !nvme.wait_ready(0, timeout.max(1000).min(10_000)) {
        blog!("nvme: controller did not stop");
        return None;
    }
    w32(bar, 0x0c, 0xffff_ffff);
    w32(bar, 0x24, (((QUEUE - 1) as u32) << 16) | (QUEUE - 1) as u32);
    w64(bar, 0x28, nvme.admin_sq);
    w64(bar, 0x30, nvme.admin_cq);
    w32(bar, 0x14, 1 | (6 << 16) | (4 << 20));
    if !nvme.wait_ready(1, timeout.max(1000).min(10_000)) {
        blog!("nvme: controller did not become ready, csts {:#x}", r32(bar, 0x1c));
        return None;
    }
    blog!("nvme: controller ready with admin queues of {} entries", QUEUE);
    let ident = nvme.scratch;
    if nvme.admin(ADMIN_IDENTIFY, 0, ident, 1, 0).is_err() {
        blog!("nvme: identify controller failed");
        return None;
    }
    unsafe {
        let bytes = ident as *const u8;
        for i in 0..40 {
            drive.model[i] = *bytes.add(24 + i);
        }
        for i in 0..8 {
            drive.firmware[i] = *bytes.add(64 + i);
        }
        drive.namespaces = read_volatile((ident + 516) as *const u32);
    }
    blog!("nvme: {} firmware {}, {} namespaces", trim(&drive.model), trim(&drive.firmware), drive.namespaces);
    unsafe { core::ptr::write_bytes(nvme.scratch as *mut u8, 0, 4096) };
    if nvme.admin(ADMIN_IDENTIFY, 1, nvme.scratch, 0, 0).is_err() {
        blog!("nvme: identify namespace 1 failed");
        return None;
    }
    unsafe {
        drive.nsze = read_volatile(nvme.scratch as *const u64);
        let flbas = *((nvme.scratch + 26) as *const u8) & 0xf;
        let lbaf = read_volatile((nvme.scratch + 128 + 4 * flbas as u64) as *const u32);
        drive.lba_shift = (lbaf >> 16) & 0xff;
    }
    nvme.lba_shift = drive.lba_shift;
    blog!("nvme: namespace 1 has {} sectors of {} bytes ({} MiB)", drive.nsze, 1u64 << drive.lba_shift, (drive.nsze << drive.lba_shift) >> 20);
    if nvme.admin(ADMIN_CREATE_CQ, 0, nvme.io_cq, (((QUEUE - 1) as u32) << 16) | 1, 1).is_err() {
        blog!("nvme: create io completion queue failed");
        return None;
    }
    if nvme.admin(ADMIN_CREATE_SQ, 0, nvme.io_sq, (((QUEUE - 1) as u32) << 16) | 1, (1 << 16) | 1).is_err() {
        blog!("nvme: create io submission queue failed");
        return None;
    }
    blog!("nvme: io queue pair created (read only driver)");
    Some(nvme)
}

pub fn trim(bytes: &[u8]) -> &str {
    let text = core::str::from_utf8(bytes).unwrap_or("?");
    text.trim_matches(|c: char| c == ' ' || c == '\0')
}
