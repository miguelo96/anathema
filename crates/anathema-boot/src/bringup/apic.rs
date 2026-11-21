use core::arch::x86_64::{__cpuid, __cpuid_count, _rdtsc};
use core::ptr::{read_volatile, write_volatile};

pub const MSR_APIC_BASE: u32 = 0x1b;
pub const MSR_TSC_DEADLINE: u32 = 0x6e0;

pub const TIMER_VECTOR: u32 = 0x40;
pub const SPURIOUS_VECTOR: u32 = 0xff;

const REG_ID: u32 = 0x20;
const REG_EOI: u32 = 0xb0;
const REG_SVR: u32 = 0xf0;
const REG_ESR: u32 = 0x280;
const REG_ICR_LOW: u32 = 0x300;
const REG_ICR_HIGH: u32 = 0x310;
const REG_LVT_TIMER: u32 = 0x320;
const REG_LVT_LINT0: u32 = 0x350;
const REG_LVT_LINT1: u32 = 0x360;
const REG_LVT_ERROR: u32 = 0x370;
const REG_TIMER_INIT: u32 = 0x380;
const REG_TIMER_CURRENT: u32 = 0x390;
const REG_TIMER_DIV: u32 = 0x3e0;

pub fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    ((high as u64) << 32) | low as u64
}

pub fn wrmsr(msr: u32, value: u64) {
    unsafe {
        core::arch::asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nomem, nostack, preserves_flags));
    }
}

pub fn rdtsc() -> u64 {
    unsafe { _rdtsc() }
}

pub fn delay_ticks(ticks: u64) {
    let start = rdtsc();
    while rdtsc().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

pub fn delay_us(tsc_hz: u64, us: u64) {
    delay_ticks(tsc_hz / 1_000_000 * us);
}

#[derive(Clone, Copy)]
pub struct Local {
    pub x2: bool,
    pub base: u64,
}

impl Local {
    pub fn detect() -> Local {
        let msr = rdmsr(MSR_APIC_BASE);
        Local { x2: msr & (1 << 10) != 0, base: msr & 0x000f_ffff_ffff_f000 }
    }

    pub fn read(&self, reg: u32) -> u32 {
        if self.x2 {
            rdmsr(0x800 + (reg >> 4)) as u32
        } else {
            unsafe { read_volatile((self.base + reg as u64) as *const u32) }
        }
    }

    pub fn write(&self, reg: u32, value: u32) {
        if self.x2 {
            wrmsr(0x800 + (reg >> 4), value as u64);
        } else {
            unsafe { write_volatile((self.base + reg as u64) as *mut u32, value) };
        }
    }

    pub fn id(&self) -> u32 {
        if self.x2 {
            self.read(REG_ID)
        } else {
            self.read(REG_ID) >> 24
        }
    }

    pub fn enable(&self) {
        let msr = rdmsr(MSR_APIC_BASE);
        if msr & (1 << 11) == 0 {
            wrmsr(MSR_APIC_BASE, msr | (1 << 11));
        }
        self.write(REG_SVR, 0x100 | SPURIOUS_VECTOR);
        self.write(REG_LVT_LINT0, 0x10000);
        self.write(REG_LVT_LINT1, 0x10000);
        self.write(REG_LVT_ERROR, 0x10000 | 0xfe);
        self.write(REG_LVT_TIMER, 0x10000 | TIMER_VECTOR);
        if !self.x2 {
            self.write(REG_ESR, 0);
        }
    }

    pub fn eoi(&self) {
        self.write(REG_EOI, 0);
    }

    pub fn ipi(&self, dest: u32, low: u32) {
        if self.x2 {
            wrmsr(0x830, ((dest as u64) << 32) | low as u64);
        } else {
            self.write(REG_ICR_HIGH, dest << 24);
            self.write(REG_ICR_LOW, low);
            let mut guard = 0u32;
            while self.read(REG_ICR_LOW) & (1 << 12) != 0 && guard < 1_000_000 {
                guard += 1;
                core::hint::spin_loop();
            }
        }
    }

    pub fn timer_stop(&self) {
        self.write(REG_LVT_TIMER, 0x10000 | TIMER_VECTOR);
        self.write(REG_TIMER_INIT, 0);
    }

    pub fn calibrate(&self, tsc_hz: u64) -> u64 {
        self.write(REG_TIMER_DIV, 0b0011);
        self.write(REG_LVT_TIMER, 0x10000 | TIMER_VECTOR);
        self.write(REG_TIMER_INIT, 0xffff_ffff);
        delay_ticks(tsc_hz / 100);
        let left = self.read(REG_TIMER_CURRENT);
        self.timer_stop();
        (0xffff_ffffu64 - left as u64) * 100
    }

    pub fn arm_oneshot(&self, ticks: u32) {
        self.write(REG_TIMER_DIV, 0b0011);
        self.write(REG_LVT_TIMER, TIMER_VECTOR);
        self.write(REG_TIMER_INIT, ticks);
    }

    pub fn arm_deadline(&self, deadline: u64) {
        self.write(REG_LVT_TIMER, (2 << 17) | TIMER_VECTOR);
        unsafe { core::arch::asm!("mfence", options(nomem, nostack)) };
        wrmsr(MSR_TSC_DEADLINE, deadline);
    }
}

pub fn has_tsc_deadline() -> bool {
    __cpuid(1).ecx & (1 << 24) != 0
}

pub fn has_arat() -> bool {
    __cpuid(6).eax & (1 << 2) != 0
}

pub fn cpu_apic_id() -> u32 {
    let max = __cpuid(0).eax;
    if max >= 0xb {
        let leaf = __cpuid_count(0xb, 0);
        if leaf.ebx & 0xffff != 0 {
            return leaf.edx;
        }
    }
    __cpuid(1).ebx >> 24
}
