use crate::acpi::Tables;
use crate::report::Report;
use alloc::format;
use alloc::string::String;
use core::ptr::read_volatile;

fn cfg(base: u64, bus: u64, dev: u64, func: u64, offset: u64) -> u32 {
    let address = base + (bus << 20) + (dev << 15) + (func << 12) + offset;
    unsafe { read_volatile(address as *const u32) }
}

fn vendor_name(id: u16) -> &'static str {
    match id {
        0x10de => "nvidia",
        0x1022 => "amd",
        0x1002 => "amd/ati",
        0x8086 => "intel",
        0x10ec => "realtek",
        0x144d => "samsung",
        0x15b7 => "sandisk",
        0x1987 => "phison",
        0x1b4b => "marvell",
        0x1d6a => "aquantia",
        0x14c3 => "mediatek",
        0x168c => "qualcomm",
        0x1912 => "renesas",
        0x1b21 => "asmedia",
        0x1106 => "via",
        0x1d97 => "maxio",
        0x126f => "silicon motion",
        0x2646 => "kingston",
        _ => "",
    }
}

fn class_name(class: u8, sub: u8, prog: u8) -> &'static str {
    match (class, sub) {
        (0x01, 0x06) => "sata",
        (0x01, 0x08) => "nvme",
        (0x01, _) => "storage",
        (0x02, 0x00) => "ethernet",
        (0x02, _) => "network",
        (0x03, _) => "display",
        (0x04, 0x03) => "hd audio",
        (0x04, _) => "multimedia",
        (0x05, _) => "memory",
        (0x06, 0x00) => "host bridge",
        (0x06, 0x01) => "isa bridge",
        (0x06, 0x04) => "pci bridge",
        (0x06, _) => "bridge",
        (0x08, 0x06) => "iommu",
        (0x08, _) => "system",
        (0x0c, 0x03) => match prog {
            0x30 => "usb xhci",
            0x20 => "usb ehci",
            0x10 => "usb ohci",
            0x00 => "usb uhci",
            _ => "usb",
        },
        (0x0c, _) => "serial bus",
        (0x0d, _) => "wireless",
        (0x10, _) => "encryption",
        (0x11, _) => "signal",
        _ => "other",
    }
}

fn nvidia_gpu(device: u16) -> &'static str {
    match device {
        0x2507 | 0x2582 | 0x2584 | 0x25a0 | 0x25a2 | 0x25a5 | 0x25a6 | 0x25a7 | 0x25a9 | 0x25ac | 0x25ad => {
            "geforce rtx 3050 family"
        }
        0x2504 | 0x2503 | 0x2520 | 0x2523 | 0x2560 | 0x2571 | 0x2486 | 0x2487 | 0x2488 => {
            "geforce rtx 30 series"
        }
        _ => "nvidia gpu",
    }
}

pub fn collect(report: &mut Report, tables: &Tables) {
    report.head("pci devices");
    if tables.ecam.is_empty() {
        report.bad(String::from("no mcfg table: pci config space not reachable through ecam"));
        return;
    }
    let mut total = 0;
    let mut gpus = 0;
    let mut xhci = 0;
    let mut nvme = 0;
    let mut iommu = 0;
    let mut isa = 0;
    for region in &tables.ecam {
        for bus in region.first_bus as u64..=region.last_bus as u64 {
            for dev in 0..32u64 {
                let first = cfg(region.base, bus, dev, 0, 0);
                if first & 0xffff == 0xffff {
                    continue;
                }
                let header = (cfg(region.base, bus, dev, 0, 0x0c) >> 16) as u8;
                let functions = if header & 0x80 != 0 { 8 } else { 1 };
                for func in 0..functions {
                    let id = cfg(region.base, bus, dev, func, 0);
                    if id & 0xffff == 0xffff {
                        continue;
                    }
                    total += 1;
                    let vendor = id as u16;
                    let device = (id >> 16) as u16;
                    let class_reg = cfg(region.base, bus, dev, func, 8);
                    let (class, sub, prog) = ((class_reg >> 24) as u8, (class_reg >> 16) as u8, (class_reg >> 8) as u8);
                    let kind = class_name(class, sub, prog);
                    match kind {
                        "usb xhci" => xhci += 1,
                        "nvme" => nvme += 1,
                        "iommu" => iommu += 1,
                        "isa bridge" => isa += 1,
                        "display" if vendor == 0x10de => gpus += 1,
                        _ => {}
                    }
                    let mut note = String::from(vendor_name(vendor));
                    if vendor == 0x10de && class == 0x03 {
                        note = String::from(nvidia_gpu(device));
                    }
                    let line = format!(
                        "{:02x}:{:02x}.{} {:04x}:{:04x} {:02x}{:02x}{:02x} {:<11} {}",
                        bus, dev, func, vendor, device, class, sub, prog, kind, note
                    );
                    if vendor == 0x10de && class == 0x03 {
                        report.say(line);
                    } else {
                        report.dim(line);
                    }
                    detail(report, region.base, bus, dev, func);
                }
            }
        }
    }
    report.say(String::new());
    report.say(format!(
        "{} functions, {} nvidia gpu, {} xhci, {} nvme, {} iommu, {} isa bridge",
        total, gpus, xhci, nvme, iommu, isa
    ));
    if gpus == 0 {
        report.warn(String::from("no nvidia display function found"));
    }
    if iommu == 0 {
        report.warn(String::from("no pci iommu function visible"));
    }
}

fn detail(report: &mut Report, base: u64, bus: u64, dev: u64, func: u64) {
    let header = (cfg(base, bus, dev, func, 0x0c) >> 16) as u8 & 0x7f;
    let command = cfg(base, bus, dev, func, 4);
    let mut text = format!("          cmd {:04x} status {:04x}", command & 0xffff, command >> 16);
    if header == 0 {
        let mut bars = String::new();
        let mut index = 0;
        while index < 6 {
            let value = cfg(base, bus, dev, func, 0x10 + 4 * index);
            if value != 0 {
                bars.push_str(&format!(" bar{}={:#x}", index, value));
                if value & 1 == 0 && (value >> 1) & 3 == 2 {
                    let high = cfg(base, bus, dev, func, 0x10 + 4 * (index + 1));
                    bars.push_str(&format!("/{:#x}", high));
                    index += 1;
                }
            }
            index += 1;
        }
        text.push_str(&bars);
    }
    if command >> 16 & 0x10 != 0 {
        let mut pointer = (cfg(base, bus, dev, func, 0x34) & 0xfc) as u64;
        let mut ids = String::new();
        let mut guard = 0;
        while pointer >= 0x40 && guard < 48 {
            let entry = cfg(base, bus, dev, func, pointer);
            ids.push_str(&format!(" {:02x}", entry & 0xff));
            pointer = ((entry >> 8) & 0xfc) as u64;
            guard += 1;
        }
        text.push_str(&format!(" caps{}", ids));
    }
    report.dim(text);
}
