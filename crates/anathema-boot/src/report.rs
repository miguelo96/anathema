use crate::{acpi, cpu, gfx::GopInfo, pci};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use uefi::mem::memory_map::MemoryMap;
use uefi::boot::MemoryType;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Head,
    Normal,
    Dim,
    Warn,
    Bad,
}

pub struct Line {
    pub tone: Tone,
    pub text: String,
}

pub struct Report {
    pub lines: Vec<Line>,
}

impl Report {
    pub fn new() -> Report {
        Report { lines: Vec::new() }
    }

    pub fn push(&mut self, tone: Tone, text: String) {
        self.lines.push(Line { tone, text });
    }

    pub fn head(&mut self, text: &str) {
        if !self.lines.is_empty() {
            self.push(Tone::Normal, String::new());
        }
        self.push(Tone::Head, String::from(text));
    }

    pub fn say(&mut self, text: String) {
        self.push(Tone::Normal, text);
    }

    pub fn dim(&mut self, text: String) {
        self.push(Tone::Dim, text);
    }

    pub fn warn(&mut self, text: String) {
        self.push(Tone::Warn, text);
    }

    pub fn bad(&mut self, text: String) {
        self.push(Tone::Bad, text);
    }

    pub fn plain(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.text);
            out.push_str("\r\n");
        }
        out
    }
}

pub const STEPS: [&str; 6] = [
    "firmware",
    "memory map",
    "acpi tables",
    "processor",
    "pci devices",
    "display",
];

pub fn build(gop: &GopInfo, step: &mut dyn FnMut(usize)) -> Report {
    let mut report = Report::new();
    report.say(String::from("anathema hardware report"));
    step(0);
    firmware(&mut report);
    step(1);
    memory(&mut report);
    step(2);
    let tables = acpi::collect(&mut report);
    step(3);
    cpu::collect(&mut report, tables.iommu_table);
    step(4);
    pci::collect(&mut report, &tables);
    step(5);
    display(&mut report, gop);
    report
}

fn firmware(report: &mut Report) {
    report.head("firmware");
    let vendor = uefi::system::firmware_vendor();
    report.say(format!("vendor        {}", vendor));
    report.say(format!("revision      {:#010x}", uefi::system::firmware_revision()));
    let rev = uefi::system::uefi_revision();
    report.say(format!("uefi          {}.{}", rev.major(), rev.minor()));
    let count = uefi::system::with_config_table(|t| t.len());
    report.say(format!("config tables {}", count));
}

fn kind(ty: MemoryType) -> &'static str {
    match ty.0 {
        0 => "reserved",
        1 => "loader code",
        2 => "loader data",
        3 => "boot code",
        4 => "boot data",
        5 => "runtime code",
        6 => "runtime data",
        7 => "conventional",
        8 => "unusable",
        9 => "acpi reclaim",
        10 => "acpi nvs",
        11 => "mmio",
        12 => "mmio port",
        13 => "pal code",
        14 => "persistent",
        15 => "unaccepted",
        _ => "other",
    }
}

fn memory(report: &mut Report) {
    report.head("memory map");
    let map = match uefi::boot::memory_map(MemoryType::LOADER_DATA) {
        Ok(map) => map,
        Err(error) => {
            report.bad(format!("memory map unavailable: {:?}", error));
            return;
        }
    };
    let mut totals = [0u64; 16];
    let mut count = 0usize;
    let mut top = 0u64;
    for desc in map.entries() {
        count += 1;
        let bytes = desc.page_count * 4096;
        let slot = (desc.ty.0 as usize).min(15);
        totals[slot] += bytes;
        top = top.max(desc.phys_start + bytes);
    }
    report.say(format!("descriptors   {}", count));
    report.say(format!("highest end   {:#x} ({} MiB)", top, top >> 20));
    for (index, total) in totals.iter().enumerate() {
        if *total > 0 {
            report.say(format!("{:<14}{} MiB", kind(MemoryType(index as u32)), total >> 20));
        }
    }
    report.say(String::new());
    report.dim(String::from("start            end              pages      type"));
    for desc in map.entries() {
        let bytes = desc.page_count * 4096;
        report.dim(format!(
            "{:#016x} {:#016x} {:<10} {}",
            desc.phys_start,
            desc.phys_start + bytes,
            desc.page_count,
            kind(desc.ty)
        ));
    }
}

fn display(report: &mut Report, gop: &GopInfo) {
    report.head("display");
    report.say(format!("mode          {}x{} ({})", gop.width, gop.height, gop.format));
    report.say(format!("stride        {} pixels", gop.stride));
    report.say(format!("framebuffer   {:#x} size {} KiB", gop.base, gop.size >> 10));
    report.say(format!("modes         {} (current index {})", gop.modes.len(), gop.current_mode));
    let mut line = String::new();
    for (index, (w, h)) in gop.modes.iter().enumerate() {
        if index > 0 && index % 6 == 0 {
            report.dim(core::mem::take(&mut line));
        }
        line.push_str(&format!("{}x{}  ", w, h));
    }
    if !line.is_empty() {
        report.dim(line);
    }
}
