use super::apic::{delay_us, rdtsc};
use super::pcicfg::{self, Bdf};
use super::Prep;
use crate::blog;
use core::ptr::{read_volatile, write_volatile};

const R_DTBA: u64 = 0x00;
const R_CMD_BASE: u64 = 0x08;
const R_EVT_BASE: u64 = 0x10;
const R_CONTROL: u64 = 0x18;
const R_EFR: u64 = 0x30;
const R_CMD_HEAD: u64 = 0x2000;
const R_CMD_TAIL: u64 = 0x2008;
const R_EVT_HEAD: u64 = 0x2010;
const R_EVT_TAIL: u64 = 0x2018;
const R_STATUS: u64 = 0x2020;

const C_EN: u64 = 1 << 0;
const C_EVT_EN: u64 = 1 << 2;
const C_CMD_EN: u64 = 1 << 12;
const S_EVT_RUN: u64 = 1 << 3;
const S_CMD_RUN: u64 = 1 << 4;

pub const DEVICES: usize = 65536;
pub const TABLE_PAGES: usize = 512;
pub const CMD_PAGES: usize = 2;
pub const EVT_PAGES: usize = 2;
pub const WORK_PAGES: usize = 1;
pub const TOTAL_PAGES: usize = TABLE_PAGES + CMD_PAGES + EVT_PAGES + WORK_PAGES;

const CMD_BYTES: u64 = (CMD_PAGES * 4096) as u64;
const EVT_BYTES: u64 = (EVT_PAGES * 4096) as u64;

pub struct Outcome {
    pub state: u8,
    pub efr: u64,
    pub devices: usize,
    pub cmd_ok: bool,
    pub enabled: bool,
    pub events: u64,
    pub note: &'static str,
}

impl Outcome {
    pub const fn new() -> Outcome {
        Outcome { state: 0, efr: 0, devices: 0, cmd_ok: false, enabled: false, events: 0, note: "not run" }
    }
}

struct Unit {
    base: u64,
    cmd: u64,
    tail: u64,
    tsc_hz: u64,
}

impl Unit {
    fn r64(&self, offset: u64) -> u64 {
        unsafe { read_volatile((self.base + offset) as *const u64) }
    }

    fn w64(&self, offset: u64, value: u64) {
        unsafe { write_volatile((self.base + offset) as *mut u64, value) }
    }

    fn wait(&self, offset: u64, mask: u64, want: u64, ms: u64) -> bool {
        let begin = rdtsc();
        let limit = self.tsc_hz / 1000 * ms;
        while rdtsc().wrapping_sub(begin) < limit {
            if self.r64(offset) & mask == want {
                return true;
            }
            core::hint::spin_loop();
        }
        self.r64(offset) & mask == want
    }

    fn push(&mut self, words: [u32; 4]) -> bool {
        let head = self.r64(R_CMD_HEAD);
        let next = (self.tail + 16) % CMD_BYTES;
        if next == head {
            return false;
        }
        unsafe {
            let slot = (self.cmd + self.tail) as *mut u32;
            for (i, word) in words.iter().enumerate() {
                write_volatile(slot.add(i), *word);
            }
            core::arch::asm!("mfence", options(nomem, nostack));
        }
        self.tail = next;
        self.w64(R_CMD_TAIL, self.tail);
        true
    }

    fn completion_wait(&mut self, store: u64, value: u64) -> bool {
        unsafe { write_volatile(store as *mut u64, 0) };
        let queued = self.push([(store as u32 & !7) | 1, (store >> 32) as u32 | (1 << 28), value as u32, (value >> 32) as u32]);
        if !queued {
            return false;
        }
        let begin = rdtsc();
        let limit = self.tsc_hz / 1000 * 200;
        while rdtsc().wrapping_sub(begin) < limit {
            if unsafe { read_volatile(store as *const u64) } == value {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn invalidate_device(&mut self, id: u16) -> bool {
        self.push([id as u32, 2 << 28, 0, 0])
    }

    fn invalidate_all(&mut self) -> bool {
        self.push([0, 8 << 28, 0, 0])
    }
}

pub fn takeover(prep: &Prep, out: &mut Outcome) {
    if prep.iommu_base == 0 || prep.iommu_mem == 0 {
        out.state = 3;
        out.note = "no ivrs unit";
        blog!("amd-vi: no unit described by the ivrs, skipping");
        return;
    }
    let at = Bdf::from_id(prep.iommu_bdf);
    let ecam = prep.ecam;
    if let Some(cap) = pcicfg::find_cap(ecam, at, 0x0f) {
        let header = pcicfg::read32(ecam, at, cap);
        let low = pcicfg::read32(ecam, at, cap + 4);
        let high = pcicfg::read32(ecam, at, cap + 8);
        blog!("amd-vi capability at {:#x}: header {:#010x}, base {:#x}, enable bit {}", cap, header, ((high as u64) << 32) | (low & !0x3fff) as u64, low & 1);
        if low & 1 == 0 {
            pcicfg::write32(ecam, at, cap + 4, low | 1);
            blog!("amd-vi base address enable bit set");
        }
    } else {
        blog!("amd-vi: no iommu capability block on {:02x}:{:02x}.{}", at.bus, at.dev, at.func);
    }
    let mut unit = Unit { base: prep.iommu_base, cmd: prep.iommu_mem + (TABLE_PAGES * 4096) as u64, tail: 0, tsc_hz: prep.tsc_hz };
    let efr = unit.r64(R_EFR);
    let control = unit.r64(R_CONTROL);
    out.efr = efr;
    if efr == u64::MAX && control == u64::MAX {
        out.state = 2;
        out.note = "mmio does not respond";
        blog!("amd-vi: mmio at {:#x} reads all ones, giving up", unit.base);
        return;
    }
    blog!("amd-vi at {:#x}: efr {:#x}, control {:#x}, status {:#x}, dtba {:#x}", unit.base, efr, control, unit.r64(R_STATUS), unit.r64(R_DTBA));
    blog!(
        "amd-vi features: xt {} nx {} gt {} ia {} ga {} he {} pc {} hats {} gats {}",
        efr & 1,
        (efr >> 1) & 1,
        (efr >> 4) & 1,
        (efr >> 6) & 1,
        (efr >> 7) & 1,
        (efr >> 8) & 1,
        (efr >> 9) & 1,
        (efr >> 10) & 3,
        (efr >> 12) & 3
    );
    if control & (C_EN | C_CMD_EN | C_EVT_EN) != 0 {
        unit.w64(R_CONTROL, control & !(C_EN | C_CMD_EN | C_EVT_EN | (1 << 13) | (1 << 14) | (1 << 15)));
        unit.wait(R_STATUS, S_CMD_RUN | S_EVT_RUN, 0, 100);
        blog!("amd-vi was enabled by firmware, disabled it first");
    }
    let table = prep.iommu_mem;
    let evt = unit.cmd + CMD_BYTES;
    let work = evt + EVT_BYTES;
    unsafe {
        let entries = table as *mut u64;
        let word0: u64 = 1 | (1 << 1) | (1 << 61) | (1 << 62);
        for device in 0..DEVICES {
            write_volatile(entries.add(device * 4), word0);
            write_volatile(entries.add(device * 4 + 1), 1);
            write_volatile(entries.add(device * 4 + 2), 0);
            write_volatile(entries.add(device * 4 + 3), 0);
        }
        core::ptr::write_bytes(unit.cmd as *mut u8, 0, (CMD_BYTES + EVT_BYTES) as usize);
        core::arch::asm!("mfence", options(nomem, nostack));
    }
    out.devices = DEVICES;
    unit.w64(R_DTBA, table | (TABLE_PAGES as u64 - 1));
    unit.w64(R_CMD_BASE, unit.cmd | (9u64 << 56));
    unit.w64(R_EVT_BASE, evt | (9u64 << 56));
    unit.w64(R_CMD_HEAD, 0);
    unit.w64(R_CMD_TAIL, 0);
    unit.w64(R_EVT_HEAD, 0);
    unit.w64(R_EVT_TAIL, 0);
    unit.w64(R_CONTROL, C_EVT_EN | C_CMD_EN | C_EN);
    delay_us(prep.tsc_hz, 1000);
    out.enabled = unit.r64(R_CONTROL) & C_EN != 0;
    let running = unit.wait(R_STATUS, S_CMD_RUN | S_EVT_RUN, S_CMD_RUN | S_EVT_RUN, 200);
    blog!("amd-vi enabled: {}, command buffer and event log running: {} (control {:#x}, status {:#x})", out.enabled, running, unit.r64(R_CONTROL), unit.r64(R_STATUS));
    if !running {
        out.state = 2;
        out.note = "queues did not start";
        return;
    }
    let first = if unit.efr_ia(efr) { unit.invalidate_all() } else { unit.invalidate_device(prep.nvme_bdf) };
    let first = first && unit.completion_wait(work, 0x414e_4154_4845_4d41);
    blog!("amd-vi self test 1, invalidate and completion wait: {}", if first { "ok" } else { "FAILED" });
    let mut second = unit.invalidate_device(prep.nvme_bdf);
    second &= unit.invalidate_device(prep.iommu_bdf);
    second &= unit.completion_wait(work, 0x4d41_5445_4841_4e41);
    blog!("amd-vi self test 2, device table invalidation and completion wait: {}", if second { "ok" } else { "FAILED" });
    let head = unit.r64(R_EVT_HEAD);
    let tail = unit.r64(R_EVT_TAIL);
    out.events = (tail.wrapping_sub(head) % EVT_BYTES) / 16;
    blog!("amd-vi event log: {} entries pending (head {:#x} tail {:#x})", out.events, head, tail);
    let mut shown = 0;
    let mut at_head = head;
    while at_head != tail && shown < 4 {
        let entry = (evt + at_head) as *const u32;
        unsafe {
            blog!(
                "amd-vi event {:#010x} {:#010x} {:#010x} {:#010x}",
                read_volatile(entry),
                read_volatile(entry.add(1)),
                read_volatile(entry.add(2)),
                read_volatile(entry.add(3))
            );
        }
        at_head = (at_head + 16) % EVT_BYTES;
        shown += 1;
    }
    let cmd_head = unit.r64(R_CMD_HEAD);
    blog!("amd-vi command head {:#x} tail {:#x}", cmd_head, unit.tail);
    out.cmd_ok = first && second && cmd_head == unit.tail;
    out.state = if out.cmd_ok && out.enabled { 1 } else { 2 };
    out.note = if out.state == 1 { "enabled, all devices identity mapped" } else { "self test failed" };
}

impl Unit {
    fn efr_ia(&self, efr: u64) -> bool {
        efr & (1 << 6) != 0
    }
}
