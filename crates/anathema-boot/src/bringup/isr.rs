use super::apic::{self, Local};
use super::isr_gen::VECTORS;
use super::{crash, CORES};
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
pub struct Frame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);
pub static TIMER_IRQS: AtomicU64 = AtomicU64::new(0);
pub static TIMER_TSC: AtomicU64 = AtomicU64::new(0);
pub static SPURIOUS: AtomicU64 = AtomicU64::new(0);

global_asm!(
    "isr_common:",
    "push rax",
    "push rcx",
    "push rdx",
    "push rbx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rdi, rsp",
    "cld",
    "call {dispatch}",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rbx",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "add rsp, 16",
    "iretq",
    dispatch = sym dispatch,
);

extern "C" {
    static isr_table: [u64; 81];
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    mid: u16,
    high: u32,
    zero: u32,
}

#[repr(C, packed)]
pub struct Pointer {
    pub limit: u16,
    pub base: u64,
}

static mut IDT: [Gate; 256] = [Gate { low: 0, selector: 0, ist: 0, flags: 0, mid: 0, high: 0, zero: 0 }; 256];

pub fn build_idt() -> Pointer {
    unsafe {
        let idt = core::ptr::addr_of_mut!(IDT) as *mut Gate;
        for (slot, vector) in VECTORS.iter().enumerate() {
            let handler = isr_table[slot];
            *idt.add(*vector as usize) = Gate {
                low: handler as u16,
                selector: 0x08,
                ist: 0,
                flags: 0x8e,
                mid: (handler >> 16) as u16,
                high: (handler >> 32) as u32,
                zero: 0,
            };
        }
        Pointer { limit: (256 * 16 - 1) as u16, base: idt as u64 }
    }
}

pub fn idt_pointer() -> Pointer {
    Pointer { limit: (256 * 16 - 1) as u16, base: core::ptr::addr_of!(IDT) as u64 }
}

pub fn name(vector: u64) -> &'static str {
    match vector {
        0 => "divide error",
        1 => "debug",
        2 => "nmi",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        10 => "invalid tss",
        11 => "segment not present",
        12 => "stack fault",
        13 => "general protection",
        14 => "page fault",
        16 => "x87 fault",
        17 => "alignment check",
        18 => "machine check",
        19 => "simd fault",
        21 => "control protection",
        _ => "exception",
    }
}

extern "sysv64" fn dispatch(frame: *mut Frame) {
    let frame = unsafe { &mut *frame };
    match frame.vector {
        3 => {
            BREAKPOINTS.fetch_add(1, Ordering::SeqCst);
        }
        0x40 => {
            TIMER_TSC.store(apic::rdtsc(), Ordering::SeqCst);
            TIMER_IRQS.fetch_add(1, Ordering::SeqCst);
            Local::detect().eoi();
        }
        0xff => {
            SPURIOUS.fetch_add(1, Ordering::SeqCst);
        }
        _ => fatal(frame),
    }
}

fn fatal(frame: &Frame) -> ! {
    let id = apic::cpu_apic_id();
    let cr2: u64;
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
    if id != super::bsp_id() {
        for slot in CORES.iter() {
            if slot.apic_id.load(Ordering::SeqCst) == id as u64 && slot.state.load(Ordering::SeqCst) != 0 {
                slot.fault.store((frame.vector << 32) | (frame.error & 0xffff_ffff), Ordering::SeqCst);
                slot.fault_rip.store(frame.rip, Ordering::SeqCst);
                slot.state.store(super::STATE_FAULT, Ordering::SeqCst);
            }
        }
        loop {
            unsafe { core::arch::asm!("cli; hlt") };
        }
    }
    crash::report(frame, cr2)
}
