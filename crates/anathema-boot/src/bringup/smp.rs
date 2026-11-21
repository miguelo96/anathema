use super::apic::{self, Local};
use super::{tables, CoreSlot, CORES, STATE_READY, STATE_STARTED};
use core::arch::global_asm;
use core::mem::offset_of;
use core::sync::atomic::Ordering;

pub const DATA: usize = 0x800;
pub const STACK_SIZE: usize = 64 * 1024;

#[repr(C)]
pub struct TrampData {
    pub cr3: u32,
    pub cr4: u32,
    pub cr0: u32,
    pub efer: u32,
    pub stack: u64,
    pub entry: u64,
    pub index: u64,
    pub gdt64: [u8; 10],
    pub pad1: [u8; 6],
    pub idt64: [u8; 10],
    pub pad2: [u8; 6],
    pub far32: [u8; 6],
    pub pad3: [u8; 2],
    pub far64: [u8; 6],
    pub pad4: [u8; 2],
    pub tgdt: [u64; 4],
    pub tgdtr: [u8; 6],
}

const TD_CR3: usize = DATA + offset_of!(TrampData, cr3);
const TD_CR4: usize = DATA + offset_of!(TrampData, cr4);
const TD_CR0: usize = DATA + offset_of!(TrampData, cr0);
const TD_EFER: usize = DATA + offset_of!(TrampData, efer);
const TD_STACK: usize = DATA + offset_of!(TrampData, stack);
const TD_ENTRY: usize = DATA + offset_of!(TrampData, entry);
const TD_INDEX: usize = DATA + offset_of!(TrampData, index);
const TD_GDT64: usize = DATA + offset_of!(TrampData, gdt64);
const TD_IDT64: usize = DATA + offset_of!(TrampData, idt64);
const TD_FAR32: usize = DATA + offset_of!(TrampData, far32);
const TD_FAR64: usize = DATA + offset_of!(TrampData, far64);
const TD_TGDTR: usize = DATA + offset_of!(TrampData, tgdtr);

global_asm!(
    ".globl ap_tramp_start",
    ".globl ap_tramp_end",
    ".globl ap_pm32",
    ".globl ap_lm64",
    ".balign 16",
    "ap_tramp_start:",
    ".code16",
    "cli",
    "cld",
    "mov ax, cs",
    "mov ds, ax",
    "xor ebx, ebx",
    "mov bx, ax",
    "shl ebx, 4",
    "lgdt [{tgdtr}]",
    "mov eax, cr0",
    "or eax, 1",
    "mov cr0, eax",
    "jmp fword ptr [{far32}]",
    ".code32",
    "ap_pm32:",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    "mov fs, ax",
    "mov gs, ax",
    "mov eax, [ebx + {cr4}]",
    "mov cr4, eax",
    "mov eax, [ebx + {cr3}]",
    "mov cr3, eax",
    "mov ecx, 0xc0000080",
    "mov eax, [ebx + {efer}]",
    "xor edx, edx",
    "wrmsr",
    "mov eax, [ebx + {cr0}]",
    "mov cr0, eax",
    "jmp fword ptr [ebx + {far64}]",
    ".code64",
    "ap_lm64:",
    "lgdt [rbx + {gdt64}]",
    "lidt [rbx + {idt64}]",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    "xor eax, eax",
    "mov fs, ax",
    "mov gs, ax",
    "mov rsp, [rbx + {stack}]",
    "mov rax, [rbx + {entry}]",
    "mov rdi, [rbx + {index}]",
    "push 0x08",
    "lea rcx, [rip + 2f]",
    "push rcx",
    "retfq",
    "2:",
    "xor ebp, ebp",
    "call rax",
    "3:",
    "cli",
    "hlt",
    "jmp 3b",
    "ap_tramp_end:",
    cr3 = const TD_CR3,
    cr4 = const TD_CR4,
    cr0 = const TD_CR0,
    efer = const TD_EFER,
    stack = const TD_STACK,
    entry = const TD_ENTRY,
    index = const TD_INDEX,
    gdt64 = const TD_GDT64,
    idt64 = const TD_IDT64,
    far32 = const TD_FAR32,
    far64 = const TD_FAR64,
    tgdtr = const TD_TGDTR,
);

extern "C" {
    static ap_tramp_start: u8;
    static ap_tramp_end: u8;
    static ap_pm32: u8;
    static ap_lm64: u8;
}

pub struct Trampoline {
    pub phys: u64,
}

pub unsafe fn install(phys: u64, cr3: u64, bsp_cr0: u64, bsp_cr4: u64, bsp_efer: u64) -> Option<Trampoline> {
    let start = core::ptr::addr_of!(ap_tramp_start) as usize;
    let end = core::ptr::addr_of!(ap_tramp_end) as usize;
    let len = end - start;
    if len >= DATA || phys >= 0x10_0000 || phys & 0xfff != 0 {
        return None;
    }
    let dest = phys as *mut u8;
    core::ptr::write_bytes(dest, 0, 4096);
    core::ptr::copy_nonoverlapping(start as *const u8, dest, len);
    let data = &mut *((phys as usize + DATA) as *mut TrampData);
    data.cr3 = cr3 as u32;
    data.cr4 = (bsp_cr4 & !(1 << 17)) as u32 | 0x20;
    data.cr0 = bsp_cr0 as u32;
    data.efer = ((bsp_efer & 0x801) | 0x100) as u32;
    data.entry = ap_entry as *const () as usize as u64;
    let gdt = tables::gdt_pointer();
    let idt = super::isr::idt_pointer();
    data.gdt64[..2].copy_from_slice(&{ gdt.limit }.to_le_bytes());
    data.gdt64[2..10].copy_from_slice(&{ gdt.base }.to_le_bytes());
    data.idt64[..2].copy_from_slice(&{ idt.limit }.to_le_bytes());
    data.idt64[2..10].copy_from_slice(&{ idt.base }.to_le_bytes());
    let pm32 = phys + (core::ptr::addr_of!(ap_pm32) as usize - start) as u64;
    let lm64 = phys + (core::ptr::addr_of!(ap_lm64) as usize - start) as u64;
    data.far32[..4].copy_from_slice(&(pm32 as u32).to_le_bytes());
    data.far32[4..6].copy_from_slice(&0x08u16.to_le_bytes());
    data.far64[..4].copy_from_slice(&(lm64 as u32).to_le_bytes());
    data.far64[4..6].copy_from_slice(&0x18u16.to_le_bytes());
    data.tgdt = [0, 0x00cf_9a00_0000_ffff, 0x00cf_9200_0000_ffff, 0x00af_9a00_0000_ffff];
    let tgdt_base = phys + (DATA + offset_of!(TrampData, tgdt)) as u64;
    data.tgdtr[..2].copy_from_slice(&((4 * 8 - 1) as u16).to_le_bytes());
    data.tgdtr[2..6].copy_from_slice(&(tgdt_base as u32).to_le_bytes());
    Some(Trampoline { phys })
}

impl Trampoline {
    pub unsafe fn start(&self, local: &Local, tsc_hz: u64, dest: u32, index: usize, stack_top: u64) -> bool {
        let data = &mut *((self.phys as usize + DATA) as *mut TrampData);
        core::ptr::write_volatile(&mut data.stack, stack_top);
        core::ptr::write_volatile(&mut data.index, index as u64);
        core::arch::asm!("mfence", options(nomem, nostack));
        let slot = &CORES[index];
        slot.state.store(0, Ordering::SeqCst);
        local.ipi(dest, 0xc500);
        apic::delay_us(tsc_hz, 10_000);
        let vector = (self.phys >> 12) as u32;
        for _ in 0..2 {
            local.ipi(dest, 0x4600 | vector);
            apic::delay_us(tsc_hz, 300);
        }
        let begin = apic::rdtsc();
        while apic::rdtsc().wrapping_sub(begin) < tsc_hz / 5 {
            if slot.state.load(Ordering::SeqCst) >= STATE_READY {
                return true;
            }
            core::hint::spin_loop();
        }
        slot.state.load(Ordering::SeqCst) >= STATE_STARTED
    }
}

extern "sysv64" fn ap_entry(index: u64) -> ! {
    let slot: &CoreSlot = &CORES[index as usize];
    let tsc = apic::rdtsc();
    let local = Local::detect();
    local.enable();
    let id = local.id();
    slot.apic_id.store(id as u64, Ordering::SeqCst);
    slot.tsc.store(tsc, Ordering::SeqCst);
    slot.state.store(STATE_STARTED, Ordering::SeqCst);
    slot.fill_cpu();
    let rsp: u64;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack)) };
    slot.rsp.store(rsp, Ordering::SeqCst);
    slot.state.store(STATE_READY, Ordering::SeqCst);
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}
