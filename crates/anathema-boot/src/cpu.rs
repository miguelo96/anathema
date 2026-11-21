use crate::report::Report;
use alloc::format;
use alloc::string::String;
use core::arch::x86_64::{__cpuid, __cpuid_count};

fn flag(report: &mut Report, name: &str, on: bool) {
    report.say(format!("{:<14}{}", name, if on { "yes" } else { "no" }));
}

pub fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack));
    }
    ((high as u64) << 32) | low as u64
}

pub fn collect(report: &mut Report, iommu_table: bool) {
    report.head("processor");
    let leaf0 = __cpuid(0);
    let mut vendor = String::new();
    for reg in [leaf0.ebx, leaf0.edx, leaf0.ecx] {
        for byte in reg.to_le_bytes() {
            vendor.push(byte as char);
        }
    }
    report.say(format!("vendor        {}", vendor));
    let max_ext = __cpuid(0x8000_0000).eax;
    if max_ext >= 0x8000_0004 {
        let mut brand = String::new();
        for leaf in 0x8000_0002..=0x8000_0004u32 {
            let r = __cpuid(leaf);
            for reg in [r.eax, r.ebx, r.ecx, r.edx] {
                for byte in reg.to_le_bytes() {
                    if byte != 0 {
                        brand.push(byte as char);
                    }
                }
            }
        }
        report.say(format!("model         {}", brand.trim()));
    }
    let leaf1 = __cpuid(1);
    let base = (leaf1.eax >> 8) & 0xf;
    let family = if base == 0xf { base + ((leaf1.eax >> 20) & 0xff) } else { base };
    let model = ((leaf1.eax >> 4) & 0xf) | (((leaf1.eax >> 16) & 0xf) << 4);
    report.say(format!(
        "family        {:#x} model {:#x} stepping {}",
        family,
        model,
        leaf1.eax & 0xf
    ));
    let leaf7 = if leaf0.eax >= 7 { __cpuid_count(7, 0) } else { __cpuid(0) };
    let ext1 = if max_ext >= 0x8000_0001 { __cpuid(0x8000_0001) } else { __cpuid(0) };
    flag(report, "sse4.2", leaf1.ecx & (1 << 20) != 0);
    flag(report, "popcnt", leaf1.ecx & (1 << 23) != 0);
    flag(report, "avx", leaf1.ecx & (1 << 28) != 0);
    flag(report, "avx2", leaf7.ebx & (1 << 5) != 0);
    flag(report, "bmi2", leaf7.ebx & (1 << 8) != 0);
    flag(report, "avx512f", leaf7.ebx & (1 << 16) != 0);
    flag(report, "la57", leaf7.ecx & (1 << 16) != 0);
    flag(report, "1 gib pages", ext1.edx & (1 << 26) != 0);
    flag(report, "nx", ext1.edx & (1 << 20) != 0);
    flag(report, "hypervisor", leaf1.ecx & (1 << 31) != 0);
    if max_ext >= 0x8000_0008 {
        let sizes = __cpuid(0x8000_0008);
        report.say(format!(
            "address bits  physical {} linear {}",
            sizes.eax & 0xff,
            (sizes.eax >> 8) & 0xff
        ));
    }
    let amd = vendor == "AuthenticAMD";
    let svm = ext1.ecx & (1 << 2) != 0;
    flag(report, "svm", svm);
    if amd && svm && max_ext >= 0x8000_000a {
        let features = __cpuid(0x8000_000a);
        report.say(format!(
            "svm revision  {} asids {} features {:#x}",
            features.eax & 0xff,
            features.ebx,
            features.edx
        ));
        flag(report, "nested paging", features.edx & 1 != 0);
        let vm_cr = rdmsr(0xc001_0114);
        let disabled = vm_cr & (1 << 4) != 0;
        report.say(format!("vm_cr         {:#x}", vm_cr));
        if disabled {
            report.warn(String::from("svm is disabled in firmware (svmdis set)"));
        } else {
            report.say(String::from("svm is enabled in firmware"));
        }
    }
    let efer = if amd { rdmsr(0xc000_0080) } else { 0 };
    if amd {
        report.say(format!("efer          {:#x}", efer));
    }
    let cr4: u64;
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    }
    report.say(format!("cr4           {:#x} (la57 active {})", cr4, cr4 & (1 << 12) != 0));
    if amd {
        report.say(format!("amd-vi table  {}", if iommu_table { "present" } else { "absent" }));
    }
}
