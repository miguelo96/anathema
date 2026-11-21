use crate::cpu::rdmsr;
use crate::pci::{cfg, ext_caps};
use crate::report::{Inventory, Report};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::x86_64::{__cpuid, __cpuid_count, _rdtsc};
use core::ptr::read_volatile;

fn ok(report: &mut Report, text: String) {
    report.say(format!("[ok]    {}", text));
}

fn watch(report: &mut Report, text: String) {
    report.warn(format!("[watch] {}", text));
}

fn block(report: &mut Report, text: String) {
    report.bad(format!("[block] {}", text));
}

fn plan(report: &mut Report, text: &str) {
    report.dim(format!("        plan: {}", text));
}

pub fn run(inv: &Inventory) -> Report {
    let mut report = Report::new();
    report.say(String::from("anathema readiness check"));
    report.dim(String::from("read only: nothing here writes to hardware"));
    root_window(&mut report, inv);
    bus_master(&mut report, inv);
    iommu(&mut report, inv);
    gpu(&mut report, inv);
    timing(&mut report, inv);
    memory_types(&mut report);
    report
}

fn root_window(report: &mut Report, inv: &Inventory) {
    report.head("root window and config access");
    if inv.tables.ecam.is_empty() {
        block(report, String::from("no mcfg window, config space only through legacy ports"));
        plan(report, "loader refuses to continue without ecam, legacy 0xcf8 access is not implemented");
        return;
    }
    let first = inv.tables.ecam.iter().map(|e| e.first_bus).min().unwrap_or(0);
    let last = inv.tables.ecam.iter().map(|e| e.last_bus).max().unwrap_or(0);
    ok(report, format!("mcfg covers buses {}..{} in {} window(s)", first, last, inv.tables.ecam.len()));
    let mut secondaries: Vec<u8> = Vec::new();
    let mut escaped = 0;
    for f in &inv.funcs {
        if f.header == 1 {
            let buses = cfg(f.base, f.bus as u64, f.dev as u64, f.func as u64, 0x18);
            let secondary = (buses >> 8) as u8;
            let subordinate = (buses >> 16) as u8;
            if secondary != 0 {
                secondaries.push(secondary);
            }
            if subordinate > last {
                escaped += 1;
                block(
                    report,
                    format!(
                        "bridge {:02x}:{:02x}.{} forwards buses up to {} past the mcfg window ({})",
                        f.bus, f.dev, f.func, subordinate, last
                    ),
                );
            }
        }
    }
    let mut roots: Vec<u8> = Vec::new();
    for f in &inv.funcs {
        if !secondaries.contains(&f.bus) && !roots.contains(&f.bus) {
            roots.push(f.bus);
        }
    }
    ok(report, format!("{} root bus(es): {:02x?}", roots.len(), roots));
    if escaped == 0 {
        ok(report, String::from("every bridge window sits inside the mcfg range"));
    }
    plan(report, "answer all ones for any bus outside the window instead of touching memory there");
}

fn bus_master(report: &mut Report, inv: &Inventory) {
    report.head("bus master on bridges and the isa bridge");
    let mut seen = 0;
    for f in &inv.funcs {
        if f.class != 0x06 {
            continue;
        }
        seen += 1;
        let master = f.command & 4 != 0;
        let label = match f.sub {
            0x00 => "host bridge",
            0x01 => "isa bridge",
            0x04 => "pci bridge",
            _ => "bridge",
        };
        let line = format!(
            "{:02x}:{:02x}.{} {:04x}:{:04x} {} bus master {} header type {}",
            f.bus, f.dev, f.func, f.vendor, f.device, label, if master { "on" } else { "off" }, f.header
        );
        if master && (f.sub == 0x01 || f.header == 1 && f.bus == 0) {
            watch(report, line);
        } else {
            ok(report, line);
        }
    }
    if seen == 0 {
        watch(report, String::from("no bridge functions found"));
    }
    plan(report, "never rewrite a bridge command register; only attach dma isolation through the iommu and accept a bus master bit that is hard wired");
}

fn iommu(report: &mut Report, inv: &Inventory) {
    report.head("amd-vi iommu");
    if !inv.tables.iommu_table {
        block(report, String::from("no ivrs or dmar table"));
        plan(report, "without an iommu gpu dma cannot be contained: refuse to start the gpu");
        return;
    }
    let pci_iommus = inv.funcs.iter().filter(|f| f.class == 0x08 && f.sub == 0x06).count();
    ok(report, format!("ivrs ivinfo {:#010x}, {} ivhd block(s), {} pci iommu function(s)", inv.tables.ivinfo, inv.tables.iommus.len(), pci_iommus));
    for unit in &inv.tables.iommus {
        if unit.base == 0 {
            watch(report, String::from("ivhd names a zero mmio base"));
            continue;
        }
        let efr = unsafe { read_volatile((unit.base + 0x30) as *const u64) };
        let control = unsafe { read_volatile((unit.base + 0x18) as *const u64) };
        let status = unsafe { read_volatile((unit.base + 0x2020) as *const u64) };
        let line = format!(
            "unit {:02x}:{:02x}.{} mmio {:#x} efr {:#x} control {:#x} status {:#x}",
            unit.device >> 8,
            (unit.device >> 3) & 0x1f,
            unit.device & 7,
            unit.base,
            efr,
            control,
            status
        );
        if control & 1 != 0 {
            watch(report, format!("{} (firmware left it enabled)", line));
        } else {
            ok(report, format!("{} (disabled, free to program)", line));
        }
    }
    plan(report, "read the unit state first, then build our own device table and command buffer; when firmware left it enabled take it over only after quiescing its tables");
}

fn gpu(report: &mut Report, inv: &Inventory) {
    report.head("gpu and pci capabilities");
    let mut found = 0;
    for f in inv.funcs.iter().filter(|f| f.vendor == 0x10de && f.class == 0x03) {
        found += 1;
        let at = format!("{:02x}:{:02x}.{} {:04x}:{:04x}", f.bus, f.dev, f.func, f.vendor, f.device);
        let decode = f.command & 2 != 0;
        let master = f.command & 4 != 0;
        if decode {
            ok(report, format!("{} memory decoding on, bus master {}", at, if master { "on" } else { "off" }));
        } else {
            watch(report, format!("{} memory decoding off", at));
            plan(report, "move to d0 and enable memory decoding before reading its registers, as an os would");
        }
        let ids = ext_caps(f.base, f.bus as u64, f.dev as u64, f.func as u64);
        let mut text = String::new();
        for id in &ids {
            text.push_str(&format!(" {:#x}", id));
        }
        ok(report, format!("extended capabilities:{}", if text.is_empty() { " none" } else { &text }));
        if ids.contains(&0x15) {
            ok(report, String::from("resizable bar capability present"));
        }
        let mut bars = Vec::new();
        let mut index = 0;
        while index < 6 {
            let value = cfg(f.base, f.bus as u64, f.dev as u64, f.func as u64, 0x10 + 4 * index);
            if value != 0 {
                bars.push((index, value));
                if value & 1 == 0 && (value >> 1) & 3 == 2 {
                    index += 1;
                }
            }
            index += 1;
        }
        ok(report, format!("{} bar(s) assigned by firmware", bars.len()));
        if bars.is_empty() {
            block(report, String::from("firmware assigned no bars to the gpu"));
        }
    }
    if found == 0 {
        block(report, String::from("no nvidia display function on the bus"));
    }
    plan(report, "census every function's capability list once, keep it read only, and only then claim the gpu");
}

fn timing(report: &mut Report, inv: &Inventory) {
    report.head("boot processor idle wait and timers");
    let ext = __cpuid(0x8000_0000).eax;
    let invariant = ext >= 0x8000_0007 && __cpuid(0x8000_0007).edx & (1 << 8) != 0;
    if invariant {
        ok(report, String::from("invariant tsc"));
    } else {
        watch(report, String::from("tsc is not invariant"));
    }
    let start = unsafe { _rdtsc() };
    uefi::boot::stall(100_000);
    let end = unsafe { _rdtsc() };
    let hz = end.wrapping_sub(start) * 10;
    if end < start {
        block(report, String::from("tsc stepped back during a 100 ms stall"));
    } else {
        ok(report, format!("tsc measured {} MHz over a 100 ms stall", hz / 1_000_000));
    }
    let leaf0 = __cpuid(0).eax;
    if leaf0 >= 7 && __cpuid_count(7, 0).ebx & 2 != 0 {
        ok(report, format!("ia32_tsc_adjust present, value {:#x}", rdmsr(0x3b)));
    } else {
        watch(report, String::from("no ia32_tsc_adjust"));
    }
    let apic = rdmsr(0x1b);
    ok(report, format!(
        "local apic base {:#x} bsp {} x2apic {} enabled {}",
        apic & 0xffff_ffff_f000,
        (apic >> 8) & 1,
        (apic >> 10) & 1,
        (apic >> 11) & 1
    ));
    let hpet = inv.tables.sigs.iter().any(|s| s == "HPET");
    if hpet {
        ok(report, String::from("hpet table present"));
    } else {
        watch(report, String::from("no hpet table"));
    }
    plan(report, "idle by polling a deadline instead of halting, call it stuck only after two whole 1 s windows with no timer tick, and restart the window when the tsc steps back");
}

fn memory_types(report: &mut Report) {
    report.head("memory types for the other cores");
    ok(report, format!("pat {:#018x}", rdmsr(0x277)));
    ok(report, format!("mtrr cap {:#x} default {:#x}", rdmsr(0xfe), rdmsr(0x2ff)));
    if __cpuid(0).ebx == 0x6874_7541 {
        ok(report, format!("amd syscfg {:#x}", rdmsr(0xc001_0010)));
    }
    plan(report, "capture these on the boot core now, start the other cores under whatever firmware holds, and name each register that differs");
}
