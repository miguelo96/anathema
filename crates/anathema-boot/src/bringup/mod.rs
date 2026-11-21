pub mod apic;
pub mod crash;
pub mod isr;
mod isr_gen;
pub mod log;
pub mod persist;
pub mod rawfb;
pub mod screen;
pub mod smp;
pub mod tables;

use crate::blog;
use crate::gfx::Screen;
use alloc::string::String;
use apic::Local;
use core::arch::x86_64::__cpuid;
use core::ptr::read_unaligned;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};

pub const STATE_STARTED: u32 = 1;
pub const STATE_FAULT: u32 = 2;
pub const STATE_READY: u32 = 3;
pub const MAX_CORES: usize = 64;

pub struct CoreSlot {
    pub state: AtomicU32,
    pub apic_id: AtomicU64,
    pub tsc: AtomicU64,
    pub ver: AtomicU64,
    pub cr0: AtomicU64,
    pub cr3: AtomicU64,
    pub cr4: AtomicU64,
    pub efer: AtomicU64,
    pub pat: AtomicU64,
    pub mtrr: AtomicU64,
    pub apicbase: AtomicU64,
    pub rsp: AtomicU64,
    pub fault: AtomicU64,
    pub fault_rip: AtomicU64,
    pub launched: AtomicU64,
}

impl CoreSlot {
    pub const fn new() -> CoreSlot {
        CoreSlot {
            state: AtomicU32::new(0),
            apic_id: AtomicU64::new(0),
            tsc: AtomicU64::new(0),
            ver: AtomicU64::new(0),
            cr0: AtomicU64::new(0),
            cr3: AtomicU64::new(0),
            cr4: AtomicU64::new(0),
            efer: AtomicU64::new(0),
            pat: AtomicU64::new(0),
            mtrr: AtomicU64::new(0),
            apicbase: AtomicU64::new(0),
            rsp: AtomicU64::new(0),
            fault: AtomicU64::new(0),
            fault_rip: AtomicU64::new(0),
            launched: AtomicU64::new(0),
        }
    }

    pub fn fill_cpu(&self) {
        let (cr0, cr3, cr4): (u64, u64, u64);
        unsafe {
            core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        }
        self.ver.store(__cpuid(1).eax as u64, Ordering::SeqCst);
        self.cr0.store(cr0, Ordering::SeqCst);
        self.cr3.store(cr3, Ordering::SeqCst);
        self.cr4.store(cr4, Ordering::SeqCst);
        self.efer.store(apic::rdmsr(0xc000_0080), Ordering::SeqCst);
        self.pat.store(apic::rdmsr(0x277), Ordering::SeqCst);
        self.mtrr.store(apic::rdmsr(0x2ff), Ordering::SeqCst);
        self.apicbase.store(apic::rdmsr(apic::MSR_APIC_BASE), Ordering::SeqCst);
    }
}

const ZERO_SLOT: CoreSlot = CoreSlot::new();
pub static CORES: [CoreSlot; MAX_CORES] = [ZERO_SLOT; MAX_CORES];
static BSP_ID: AtomicU32 = AtomicU32::new(0);

pub fn bsp_id() -> u32 {
    BSP_ID.load(Ordering::SeqCst)
}

pub struct Prep {
    pub tsc_hz: u64,
    pub ids: [u32; MAX_CORES],
    pub enabled: usize,
    pub listed: usize,
    pub lapic: u64,
    pub hpet: u64,
    pub tramp: u64,
    pub tables: u64,
    pub stacks: u64,
    pub stack_pages: usize,
    pub bsp_stack: u64,
    pub fb_base: u64,
    pub fb_stride: usize,
    pub fb_w: usize,
    pub fb_h: usize,
    pub fb_bgr: bool,
    pub shadow: u64,
    pub cores: usize,
}

pub static mut PREP: Prep = Prep {
    tsc_hz: 0,
    ids: [0; MAX_CORES],
    enabled: 0,
    listed: 0,
    lapic: 0,
    hpet: 0,
    tramp: 0,
    tables: 0,
    stacks: 0,
    stack_pages: 0,
    bsp_stack: 0,
    fb_base: 0,
    fb_stride: 0,
    fb_w: 0,
    fb_h: 0,
    fb_bgr: true,
    shadow: 0,
    cores: 0,
};

pub struct Results {
    pub int3: bool,
    pub lapic_hz: u64,
    pub irq_us: u64,
    pub irq_ok: bool,
    pub deadline: u8,
    pub deadline_us: u64,
    pub hpet_hz: u64,
    pub hpet_tsc_hz: u64,
    pub apic_x2: bool,
    pub apic_base: u64,
    pub apic_id: u32,
    pub paging: Option<tables::PagingStats>,
    pub aps_started: usize,
    pub aps_ok: usize,
    pub arat: bool,
    pub boot_tsc: u64,
}

pub static mut RESULTS: Results = Results {
    int3: false,
    lapic_hz: 0,
    irq_us: 0,
    irq_ok: false,
    deadline: 0,
    deadline_us: 0,
    hpet_hz: 0,
    hpet_tsc_hz: 0,
    apic_x2: false,
    apic_base: 0,
    apic_id: 0,
    paging: None,
    aps_started: 0,
    aps_ok: 0,
    arat: false,
    boot_tsc: 0,
};

pub fn prep() -> &'static Prep {
    unsafe { &*core::ptr::addr_of!(PREP) }
}

pub fn results() -> &'static mut Results {
    unsafe { &mut *core::ptr::addr_of_mut!(RESULTS) }
}

unsafe fn rd<T: Copy>(address: u64) -> T {
    read_unaligned(address as *const T)
}

fn find_table(sig: &[u8; 4]) -> Option<u64> {
    let (v2, v1) = uefi::system::with_config_table(|table| {
        let (mut a, mut b) = (0u64, 0u64);
        for entry in table {
            if entry.guid == ACPI2_GUID {
                a = entry.address as u64;
            } else if entry.guid == ACPI_GUID {
                b = entry.address as u64;
            }
        }
        (a, b)
    });
    let rsdp = if v2 != 0 { v2 } else { v1 };
    if rsdp == 0 {
        return None;
    }
    unsafe {
        let revision: u8 = rd(rsdp + 15);
        let xsdt: u64 = if revision >= 2 { rd(rsdp + 24) } else { 0 };
        let rsdt: u32 = rd(rsdp + 16);
        let (root, wide) = if xsdt != 0 { (xsdt, true) } else { (rsdt as u64, false) };
        if root == 0 {
            return None;
        }
        let len: u32 = rd(root + 4);
        let step = if wide { 8 } else { 4 };
        let count = (len as u64).saturating_sub(36) / step;
        for i in 0..count {
            let entry: u64 = if wide { rd(root + 36 + step * i) } else { rd::<u32>(root + 36 + step * i) as u64 };
            if entry != 0 {
                let found: [u8; 4] = rd(entry);
                if &found == sig {
                    return Some(entry);
                }
            }
        }
    }
    None
}

fn parse_madt(prep: &mut Prep) {
    let Some(madt) = find_table(b"APIC") else {
        return;
    };
    unsafe {
        let len: u32 = rd(madt + 4);
        prep.lapic = rd::<u32>(madt + 36) as u64;
        let mut offset = 44u64;
        while offset + 2 <= len as u64 {
            let kind: u8 = rd(madt + offset);
            let size: u8 = rd(madt + offset + 1);
            if size < 2 {
                break;
            }
            match kind {
                0 => {
                    let id: u8 = rd(madt + offset + 3);
                    let flags: u32 = rd(madt + offset + 4);
                    prep.listed += 1;
                    if flags & 1 == 1 && prep.enabled < MAX_CORES {
                        prep.ids[prep.enabled] = id as u32;
                        prep.enabled += 1;
                    }
                }
                9 => {
                    let id: u32 = rd(madt + offset + 4);
                    let flags: u32 = rd(madt + offset + 8);
                    prep.listed += 1;
                    if flags & 1 == 1 && prep.enabled < MAX_CORES {
                        prep.ids[prep.enabled] = id;
                        prep.enabled += 1;
                    }
                }
                5 => prep.lapic = rd::<u64>(madt + offset + 4),
                _ => {}
            }
            offset += size as u64;
        }
    }
}

fn calibrate_tsc() -> u64 {
    let mut best = 0;
    for _ in 0..3 {
        let a = apic::rdtsc();
        boot::stall(100_000);
        let b = apic::rdtsc();
        best = (b - a) * 10;
    }
    best
}

fn alloc(ty: AllocateType, pages: usize) -> Option<u64> {
    boot::allocate_pages(ty, MemoryType::LOADER_DATA, pages).ok().map(|p| p.as_ptr() as u64)
}

pub fn run(screen: &mut Screen) -> String {
    persist::clear();
    blog!("anathema bring-up test");
    blog!("firmware {} rev {:#x}", uefi::system::firmware_vendor(), uefi::system::firmware_revision());
    let prep = unsafe { &mut *core::ptr::addr_of_mut!(PREP) };
    parse_madt(prep);
    if prep.enabled == 0 {
        return String::from("bring-up test: no enabled cpus found in the madt");
    }
    let me = apic::cpu_apic_id();
    BSP_ID.store(me, Ordering::SeqCst);
    blog!("madt: {} cpus listed, {} enabled, lapic {:#x}, boot cpu apic id {}", prep.listed, prep.enabled, prep.lapic, me);
    if let Some(hpet) = find_table(b"HPET") {
        prep.hpet = unsafe { rd::<u64>(hpet + 44) };
        blog!("hpet table: base {:#x}", prep.hpet);
    }
    prep.tsc_hz = calibrate_tsc();
    blog!("tsc calibrated against firmware stall: {} Hz ({} MHz)", prep.tsc_hz, prep.tsc_hz / 1_000_000);
    let mut ordered = [0u32; MAX_CORES];
    let mut count = 0;
    ordered[count] = me;
    count += 1;
    for i in 0..prep.enabled {
        if prep.ids[i] != me && count < MAX_CORES {
            ordered[count] = prep.ids[i];
            count += 1;
        }
    }
    prep.ids = ordered;
    prep.cores = count;
    let Some(tramp) = alloc(AllocateType::MaxAddress(0x9f000), 1) else {
        return String::from("bring-up test: no free page below 1 MiB for the ap trampoline");
    };
    let Some(tables) = alloc(AllocateType::MaxAddress(0xffff_f000), tables::TABLE_PAGES) else {
        return String::from("bring-up test: no page table memory below 4 GiB");
    };
    prep.stack_pages = smp::STACK_SIZE / 4096;
    let Some(stacks) = alloc(AllocateType::AnyPages, prep.stack_pages * (count + 1)) else {
        return String::from("bring-up test: no memory for core stacks");
    };
    prep.tramp = tramp;
    prep.tables = tables;
    prep.stacks = stacks;
    prep.bsp_stack = stacks + (smp::STACK_SIZE * (count + 1)) as u64 - 16;
    blog!("trampoline page {:#x}, page tables {:#x}, stacks {:#x} ({} x {} KiB)", tramp, tables, stacks, count + 1, smp::STACK_SIZE / 1024);
    let info = &screen.info;
    if info.format != "blt only" {
        let bytes = info.width * info.height * 4;
        if let Some(shadow) = alloc(AllocateType::AnyPages, bytes.div_ceil(4096)) {
            prep.shadow = shadow;
            prep.fb_base = info.base as u64;
            prep.fb_stride = info.stride;
            prep.fb_w = info.width;
            prep.fb_h = info.height;
            prep.fb_bgr = info.format == "bgr";
            blog!("framebuffer {:#x} {}x{} stride {} {}", prep.fb_base, prep.fb_w, prep.fb_h, prep.fb_stride, info.format);
        }
    } else {
        blog!("no linear framebuffer, status screen disabled");
    }
    blog!("leaving boot services now");
    persist::save(log::text());
    screen.clear();
    let s = screen.scale();
    screen.text_centred(screen.h / 2 - 10 * s, "BRING-UP TEST", s * 2, crate::gfx::EMBER);
    screen.text_centred(screen.h / 2 + 12 * s, "leaving uefi boot services, the machine restarts when done", s, crate::gfx::DIM);
    screen.present();
    boot::stall(1_500_000);
    let map = unsafe { boot::exit_boot_services(None) };
    unsafe { core::arch::asm!("cli") };
    let ram = tables::Ram::collect(&map);
    core::mem::forget(map);
    unsafe {
        let stats = tables::build_paging(prep.tables, &ram);
        results().paging = Some(stats);
        tables::load_gdt();
        isr::build_idt();
        tables::load_idt();
        tables::load_cr3(prep.tables);
        on_stack(prep.bsp_stack, boot_core);
    }
}

unsafe fn on_stack(top: u64, f: extern "sysv64" fn() -> !) -> ! {
    core::arch::asm!("mov rsp, {top}", "xor ebp, ebp", "call {f}", top = in(reg) top, f = in(reg) f, options(noreturn));
}

extern "sysv64" fn boot_core() -> ! {
    let prep = prep();
    let res = results();
    res.boot_tsc = apic::rdtsc();
    let fb = if prep.shadow != 0 {
        Some(rawfb::Fb {
            base: prep.fb_base as *mut u8,
            stride: prep.fb_stride,
            w: prep.fb_w,
            h: prep.fb_h,
            bgr: prep.fb_bgr,
            shadow: prep.shadow as *mut u32,
        })
    } else {
        None
    };
    crash::set_fb(fb);
    let stats = res.paging.as_ref().unwrap();
    blog!(
        "own environment up: gdt, idt, cr3 {:#x}; {} x 2 MiB ({} uncached), {} x 1 GiB ({} uncached), ram top {:#x} in {} ranges",
        stats.cr3, stats.pages_2m, stats.pages_2m_uc, stats.pages_1g, stats.pages_1g_uc, stats.ram_top, stats.ram_ranges
    );
    let slot = &CORES[0];
    slot.fill_cpu();
    let rsp: u64;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack)) };
    slot.rsp.store(rsp, Ordering::SeqCst);
    unsafe { core::arch::asm!("int3") };
    res.int3 = isr::BREAKPOINTS.load(Ordering::SeqCst) == 1;
    blog!("int3 through own idt: {}", if res.int3 { "ok" } else { "FAILED" });
    persist::save(log::text());
    let local = Local::detect();
    local.enable();
    res.apic_x2 = local.x2;
    res.apic_base = local.base;
    res.apic_id = local.id();
    res.arat = apic::has_arat();
    blog!(
        "local apic: {} mode, base {:#x}, id {} (cpuid says {}), arat {}",
        if local.x2 { "x2apic" } else { "xapic" },
        local.base,
        res.apic_id,
        apic::cpu_apic_id(),
        res.arat
    );
    slot.apic_id.store(res.apic_id as u64, Ordering::SeqCst);
    slot.tsc.store(apic::rdtsc(), Ordering::SeqCst);
    slot.state.store(STATE_READY, Ordering::SeqCst);
    timers(&local, prep, res);
    persist::save(log::text());
    smp_start(&local, prep, res);
    persist::save(log::text());
    screen::draw_status();
    persist::save(log::text());
    crash::countdown_and_reset(20)
}

fn timers(local: &Local, prep: &Prep, res: &mut Results) {
    res.lapic_hz = local.calibrate(prep.tsc_hz);
    blog!("lapic timer (divide 16) calibrated: {} Hz", res.lapic_hz);
    let ticks = (res.lapic_hz / 1000).max(1) as u32;
    let before = isr::TIMER_IRQS.load(Ordering::SeqCst);
    let armed = apic::rdtsc();
    local.arm_oneshot(ticks);
    unsafe { core::arch::asm!("sti") };
    while isr::TIMER_IRQS.load(Ordering::SeqCst) == before && apic::rdtsc().wrapping_sub(armed) < prep.tsc_hz / 20 {
        core::hint::spin_loop();
    }
    unsafe { core::arch::asm!("cli") };
    res.irq_ok = isr::TIMER_IRQS.load(Ordering::SeqCst) > before;
    if res.irq_ok {
        let at = isr::TIMER_TSC.load(Ordering::SeqCst);
        res.irq_us = at.wrapping_sub(armed) * 1_000_000 / prep.tsc_hz;
    }
    local.timer_stop();
    blog!("lapic one-shot interrupt (asked 1000 us): {} after {} us", if res.irq_ok { "delivered" } else { "NOT delivered" }, res.irq_us);
    if apic::has_tsc_deadline() {
        let before = isr::TIMER_IRQS.load(Ordering::SeqCst);
        let armed = apic::rdtsc();
        local.arm_deadline(armed + prep.tsc_hz / 1000);
        unsafe { core::arch::asm!("sti") };
        while isr::TIMER_IRQS.load(Ordering::SeqCst) == before && apic::rdtsc().wrapping_sub(armed) < prep.tsc_hz / 20 {
            core::hint::spin_loop();
        }
        unsafe { core::arch::asm!("cli") };
        if isr::TIMER_IRQS.load(Ordering::SeqCst) > before {
            res.deadline = 1;
            res.deadline_us = isr::TIMER_TSC.load(Ordering::SeqCst).wrapping_sub(armed) * 1_000_000 / prep.tsc_hz;
        } else {
            res.deadline = 2;
        }
        local.timer_stop();
        apic::wrmsr(apic::MSR_TSC_DEADLINE, 0);
        blog!("tsc-deadline timer: {} ({} us)", if res.deadline == 1 { "delivered" } else { "NOT delivered" }, res.deadline_us);
    } else {
        res.deadline = 3;
        blog!("tsc-deadline timer: not supported by this cpu, using the calibrated lapic timer");
    }
    if prep.hpet != 0 {
        unsafe {
            let caps: u64 = core::ptr::read_volatile(prep.hpet as *const u64);
            let period = caps >> 32;
            if period != 0 {
                res.hpet_hz = 1_000_000_000_000_000 / period;
                let config: u64 = core::ptr::read_volatile((prep.hpet + 0x10) as *const u64);
                if config & 1 == 0 {
                    core::ptr::write_volatile((prep.hpet + 0x10) as *mut u64, config | 1);
                }
                let h0: u64 = core::ptr::read_volatile((prep.hpet + 0xf0) as *const u64);
                let t0 = apic::rdtsc();
                apic::delay_ticks(prep.tsc_hz / 20);
                let h1: u64 = core::ptr::read_volatile((prep.hpet + 0xf0) as *const u64);
                let t1 = apic::rdtsc();
                let dh = h1.wrapping_sub(h0) & 0xffff_ffff;
                if dh != 0 {
                    res.hpet_tsc_hz = ((t1 - t0) as u128 * res.hpet_hz as u128 / dh as u128) as u64;
                }
                blog!("hpet {} Hz, tsc measured against it: {} Hz (firmware stall gave {} Hz)", res.hpet_hz, res.hpet_tsc_hz, prep.tsc_hz);
            }
        }
    }
}

fn smp_start(local: &Local, prep: &Prep, res: &mut Results) {
    let stats = res.paging.as_ref().unwrap();
    let cr0: u64;
    let cr4: u64;
    unsafe {
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    }
    let efer = apic::rdmsr(0xc000_0080);
    let tramp = unsafe { smp::install(prep.tramp, stats.cr3, cr0, cr4, efer) };
    let Some(tramp) = tramp else {
        blog!("trampoline install failed");
        return;
    };
    blog!("trampoline installed at {:#x}, sipi vector {:#x}", tramp.phys, tramp.phys >> 12);
    for index in 1..prep.cores {
        let id = prep.ids[index];
        let top = prep.stacks + (smp::STACK_SIZE * (index + 1)) as u64 - 16;
        CORES[index].launched.store(apic::rdtsc(), Ordering::SeqCst);
        let ok = unsafe { tramp.start(local, prep.tsc_hz, id, index, top) };
        res.aps_started += 1;
        let state = CORES[index].state.load(Ordering::SeqCst);
        if state == STATE_READY {
            res.aps_ok += 1;
        }
        blog!("ap {} (apic id {}): {}", index, id, if ok && state == STATE_READY { "up" } else if state == 0 { "no response" } else { "started but did not finish" });
    }
    blog!("{} of {} application processors came up", res.aps_ok, prep.cores - 1);
    for index in 0..prep.cores {
        let slot = &CORES[index];
        let ver = slot.ver.load(Ordering::SeqCst) as u32;
        let base = (ver >> 8) & 0xf;
        let family = if base == 0xf { base + ((ver >> 20) & 0xff) } else { base };
        let model = ((ver >> 4) & 0xf) | (((ver >> 16) & 0xf) << 4);
        blog!(
            "core {:>2} apic {:>3} state {} fam {:#x} model {:#x} step {} tsc {:#x} cr3 {:#x} cr4 {:#x} efer {:#x} mtrr {:#x} pat {:#x} rsp {:#x}",
            index,
            slot.apic_id.load(Ordering::SeqCst),
            slot.state.load(Ordering::SeqCst),
            family,
            model,
            ver & 0xf,
            slot.tsc.load(Ordering::SeqCst),
            slot.cr3.load(Ordering::SeqCst),
            slot.cr4.load(Ordering::SeqCst),
            slot.efer.load(Ordering::SeqCst),
            slot.mtrr.load(Ordering::SeqCst),
            slot.pat.load(Ordering::SeqCst),
            slot.rsp.load(Ordering::SeqCst)
        );
    }
}
