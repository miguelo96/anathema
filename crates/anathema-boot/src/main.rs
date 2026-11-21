#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod bringup;
mod cpu;
mod gfx;
mod input;
mod pci;
mod readiness;
mod report;
mod store;
mod ui;

use alloc::format;
use alloc::string::String;
use gfx::Screen;
use input::Key;
use uefi::prelude::*;
use uefi::runtime::{self, ResetType};

const ENTRIES: [ui::Entry; 6] = [
    ui::Entry { label: "Hardware report", hint: "memory, acpi, cpu, pci, display", enabled: true },
    ui::Entry { label: "Readiness check", hint: "blockers found on this pc and our plan", enabled: true },
    ui::Entry { label: "Bring-up test", hint: "gdt, idt, paging, apic, smp", enabled: true },
    ui::Entry { label: "Boot macOS", hint: "not available in this build", enabled: false },
    ui::Entry { label: "Restart", hint: "reboot the machine", enabled: true },
    ui::Entry { label: "Power off", hint: "shut the machine down", enabled: true },
];

#[entry]
fn main() -> Status {
    let _ = boot::set_watchdog_timer(0, 0x10000, None);
    let mut screen = match Screen::open() {
        Some(screen) => screen,
        None => {
            uefi::println!("anathema: no graphics output protocol, cannot show the menu");
            boot::stall(5_000_000);
            return Status::UNSUPPORTED;
        }
    };
    let subtitle = format!("boot loader {}", ui::VERSION);
    let info = screen.info.clone();
    let (mut report, inventory) = {
        let mut step = |done: usize| ui::splash(&mut screen, done, "reading hardware");
        report::build(&info, &mut step)
    };
    let mut recovered = false;
    if let Some(bytes) = bringup::persist::take() {
        report.head("last bring-up test");
        for line in String::from_utf8_lossy(&bytes).lines() {
            report.say(String::from(line));
        }
        recovered = true;
    }
    let readiness = readiness::run(&inventory);
    let mut full = report.plain();
    full.push_str("\r\n");
    full.push_str(&readiness.plain());
    let mut note = store::save(&full);
    if recovered {
        note = format!("bring-up log recovered and {}", note);
    }
    let mut selected = 0usize;
    loop {
        ui::menu(&mut screen, &ENTRIES, &subtitle, &note, selected);
        match input::next() {
            Key::Up => selected = (selected + ENTRIES.len() - 1) % ENTRIES.len(),
            Key::Down => selected = (selected + 1) % ENTRIES.len(),
            Key::Enter => match selected {
                0 => {
                    ui::viewer(&mut screen, "HARDWARE REPORT", &report, &full);
                    note = String::new();
                }
                1 => {
                    ui::viewer(&mut screen, "READINESS CHECK", &readiness, &full);
                    note = String::new();
                }
                2 => note = bringup::run(&mut screen),
                4 => runtime::reset(ResetType::COLD, Status::SUCCESS, None),
                5 => runtime::reset(ResetType::SHUTDOWN, Status::SUCCESS, None),
                _ => note = String::from("macOS boot is not part of this build yet"),
            },
            _ => {}
        }
    }
}
