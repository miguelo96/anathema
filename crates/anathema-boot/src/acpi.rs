use crate::report::Report;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::read_unaligned;
use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID, SMBIOS3_GUID, SMBIOS_GUID};

pub struct Ecam {
    pub base: u64,
    pub segment: u16,
    pub first_bus: u8,
    pub last_bus: u8,
}

pub struct Iommu {
    pub device: u16,
    pub cap: u16,
    pub base: u64,
    pub segment: u16,
}

pub struct Tables {
    pub ecam: Vec<Ecam>,
    pub iommu_table: bool,
    pub iommus: Vec<Iommu>,
    pub sigs: Vec<String>,
    pub ivinfo: u32,
}

unsafe fn rd<T: Copy>(address: u64) -> T {
    read_unaligned(address as *const T)
}

fn signature(address: u64) -> String {
    let bytes: [u8; 4] = unsafe { rd(address) };
    bytes
        .iter()
        .map(|b| if b.is_ascii_graphic() { *b as char } else { '?' })
        .collect()
}

fn text(address: u64, len: usize) -> String {
    let mut out = String::new();
    for i in 0..len {
        let b: u8 = unsafe { rd(address + i as u64) };
        out.push(if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' });
    }
    out
}

pub fn collect(report: &mut Report) -> Tables {
    report.head("acpi and system tables");
    let (mut_rsdp, found) = uefi::system::with_config_table(|table| {
        let mut found: Vec<String> = Vec::new();
        let mut rsdp = [0u64; 2];
        for entry in table {
            if entry.guid == ACPI2_GUID {
                rsdp[0] = entry.address as u64;
            } else if entry.guid == ACPI_GUID {
                rsdp[1] = entry.address as u64;
            } else if entry.guid == SMBIOS3_GUID {
                found.push(format!("smbios 3      {:#x}", entry.address as u64));
            } else if entry.guid == SMBIOS_GUID {
                found.push(format!("smbios        {:#x}", entry.address as u64));
            }
        }
        (rsdp, found)
    });
    for line in found {
        report.say(line);
    }
    let mut rsdp = mut_rsdp[0];
    let rsdp_old = mut_rsdp[1];
    let mut tables = Tables { ecam: Vec::new(), iommu_table: false, iommus: Vec::new(), sigs: Vec::new(), ivinfo: 0 };
    if rsdp == 0 {
        rsdp = rsdp_old;
    }
    if rsdp == 0 {
        report.bad(String::from("no acpi rsdp in the configuration tables"));
        return tables;
    }
    let revision: u8 = unsafe { rd(rsdp + 15) };
    let rsdt: u32 = unsafe { rd(rsdp + 16) };
    let xsdt: u64 = if revision >= 2 { unsafe { rd(rsdp + 24) } } else { 0 };
    report.say(format!("rsdp          {:#x} revision {} oem {}", rsdp, revision, text(rsdp + 9, 6)));
    let mut addresses: Vec<u64> = Vec::new();
    if xsdt != 0 {
        let len: u32 = unsafe { rd(xsdt + 4) };
        report.say(format!("xsdt          {:#x} {} bytes", xsdt, len));
        let count = (len as usize).saturating_sub(36) / 8;
        for i in 0..count {
            addresses.push(unsafe { rd(xsdt + 36 + 8 * i as u64) });
        }
    } else if rsdt != 0 {
        let len: u32 = unsafe { rd(rsdt as u64 + 4) };
        report.say(format!("rsdt          {:#x} {} bytes", rsdt, len));
        let count = (len as usize).saturating_sub(36) / 4;
        for i in 0..count {
            let entry: u32 = unsafe { rd(rsdt as u64 + 36 + 4 * i as u64) };
            addresses.push(entry as u64);
        }
    }
    report.say(format!("tables        {}", addresses.len()));
    report.dim(String::from("sig  address          rev  length  oem"));
    for &address in &addresses {
        if address == 0 {
            continue;
        }
        let sig = signature(address);
        let len: u32 = unsafe { rd(address + 4) };
        let rev: u8 = unsafe { rd(address + 8) };
        report.dim(format!(
            "{}  {:#014x}  {:<3}  {:<6}  {}",
            sig,
            address,
            rev,
            len,
            text(address + 10, 6)
        ));
        tables.sigs.push(sig.clone());
        match sig.as_str() {
            "MCFG" => mcfg(report, address, len, &mut tables),
            "APIC" => madt(report, address, len),
            "IVRS" => {
                tables.iommu_table = true;
                ivrs(report, address, len, &mut tables);
            }
            "DMAR" => {
                tables.iommu_table = true;
                report.say(String::from("dmar          intel vt-d table present"));
            }
            _ => {}
        }
    }
    if !tables.iommu_table {
        report.warn(String::from("no ivrs or dmar table: firmware exposes no iommu"));
    }
    tables
}

fn mcfg(report: &mut Report, address: u64, len: u32, tables: &mut Tables) {
    let count = (len as usize).saturating_sub(44) / 16;
    for i in 0..count {
        let at = address + 44 + 16 * i as u64;
        let base: u64 = unsafe { rd(at) };
        let segment: u16 = unsafe { rd(at + 8) };
        let first_bus: u8 = unsafe { rd(at + 10) };
        let last_bus: u8 = unsafe { rd(at + 11) };
        report.say(format!(
            "mcfg          ecam {:#x} segment {} buses {}..{}",
            base, segment, first_bus, last_bus
        ));
        tables.ecam.push(Ecam { base, segment, first_bus, last_bus });
    }
}

fn madt(report: &mut Report, address: u64, len: u32) {
    let local: u32 = unsafe { rd(address + 36) };
    let mut offset = 44u64;
    let (mut cpus, mut enabled, mut ioapics) = (0, 0, 0);
    while offset + 2 <= len as u64 {
        let kind: u8 = unsafe { rd(address + offset) };
        let size: u8 = unsafe { rd(address + offset + 1) };
        if size < 2 {
            break;
        }
        match kind {
            0 => {
                cpus += 1;
                let flags: u32 = unsafe { rd(address + offset + 4) };
                if flags & 1 == 1 {
                    enabled += 1;
                }
            }
            9 => {
                cpus += 1;
                let flags: u32 = unsafe { rd(address + offset + 8) };
                if flags & 1 == 1 {
                    enabled += 1;
                }
            }
            1 => ioapics += 1,
            _ => {}
        }
        offset += size as u64;
    }
    report.say(format!(
        "madt          local apic {:#x}, {} cpus ({} enabled), {} io apics",
        local, cpus, enabled, ioapics
    ));
}

fn ivrs(report: &mut Report, address: u64, len: u32, tables: &mut Tables) {
    let info: u32 = unsafe { rd(address + 36) };
    tables.ivinfo = info;
    report.say(format!(
        "ivrs          ivinfo {:#010x} efr {} pa bits {} va bits {}",
        info,
        info & 1,
        (info >> 8) & 0x7f,
        (info >> 15) & 0x7f
    ));
    let mut offset = 48u64;
    while offset + 4 <= len as u64 {
        let kind: u8 = unsafe { rd(address + offset) };
        let size: u16 = unsafe { rd(address + offset + 2) };
        if size < 4 {
            break;
        }
        if kind == 0x10 || kind == 0x11 || kind == 0x40 {
            let device: u16 = unsafe { rd(address + offset + 4) };
            let cap: u16 = unsafe { rd(address + offset + 6) };
            let base: u64 = unsafe { rd(address + offset + 8) };
            let segment: u16 = unsafe { rd(address + offset + 16) };
            report.say(format!(
                "ivhd {:#04x}     iommu {:02x}:{:02x}.{} cap {:#x} mmio {:#x} segment {}",
                kind,
                device >> 8,
                (device >> 3) & 0x1f,
                device & 7,
                cap,
                base,
                segment
            ));
            tables.iommus.push(Iommu { device, cap, base, segment });
        }
        offset += size as u64;
    }
}
