use super::isr::{self, Frame};
use super::rawfb::Fb;
use super::{apic, log, persist, prep};
use crate::blog;
use crate::gfx::{DIM, EMBER, RED, TEXT};
use uefi::runtime::{self, ResetType};
use uefi::Status;

static mut FB: Option<Fb> = None;

pub fn set_fb(fb: Option<Fb>) {
    unsafe { *core::ptr::addr_of_mut!(FB) = fb };
}

pub fn fb() -> Option<&'static mut Fb> {
    unsafe { (*core::ptr::addr_of_mut!(FB)).as_mut() }
}

pub fn report(frame: &Frame, cr2: u64) -> ! {
    blog!(
        "EXCEPTION {} ({}) error {:#x} on apic id {}",
        frame.vector,
        isr::name(frame.vector),
        frame.error,
        apic::cpu_apic_id()
    );
    blog!("rip {:#018x} cs {:#x} rflags {:#x} rsp {:#018x} cr2 {:#018x}", frame.rip, frame.cs, frame.rflags, frame.rsp, cr2);
    blog!(
        "rax {:#018x} rbx {:#018x} rcx {:#018x} rdx {:#018x}",
        frame.rax,
        frame.rbx,
        frame.rcx,
        frame.rdx
    );
    blog!(
        "rsi {:#018x} rdi {:#018x} rbp {:#018x} r8  {:#018x}",
        frame.rsi,
        frame.rdi,
        frame.rbp,
        frame.r8
    );
    blog!(
        "r9  {:#018x} r10 {:#018x} r11 {:#018x} r12 {:#018x}",
        frame.r9,
        frame.r10,
        frame.r11,
        frame.r12
    );
    blog!("r13 {:#018x} r14 {:#018x} r15 {:#018x}", frame.r13, frame.r14, frame.r15);
    if let Some(fb) = fb() {
        fb.clear();
        let top = fb.header("exception in the bring-up environment");
        let s = fb.scale();
        let lines = log::text();
        let mut start = lines.len();
        let mut seen = 0;
        while start > 0 && seen < 14 {
            start -= 1;
            if lines[start] == b'\n' {
                seen += 1;
            }
        }
        let mut y = top + 8 * s;
        let mut line = log::Line::new();
        for &byte in &lines[start..] {
            if byte == b'\n' {
                fb.text(12 * s, y, line.as_str(), s, RED);
                y += 10 * s;
                line.len = 0;
            } else if byte != b'\r' && line.len < line.buf.len() {
                line.buf[line.len] = byte;
                line.len += 1;
            }
        }
        fb.flush();
    }
    persist::save(log::text());
    countdown_and_reset(20)
}

pub fn countdown_and_reset(seconds: u64) -> ! {
    let hz = prep().tsc_hz.max(1_000_000_000);
    for left in (1..=seconds).rev() {
        if let Some(fb) = fb() {
            let s = fb.scale();
            let y = fb.h.saturating_sub(18 * s);
            fb.fill(0, y, fb.w, 12 * s, super::rawfb::BG);
            let mut line = log::Line::new();
            let _ = core::fmt::Write::write_fmt(&mut line, format_args!("warm reset in {} s, the log is saved for the next boot", left));
            let w = line.len * 8 * s;
            fb.text(fb.w.saturating_sub(w) / 2, y, line.as_str(), s, if left > 5 { DIM } else { EMBER });
            fb.flush_rows(y, y + 12 * s);
        }
        apic::delay_ticks(hz);
    }
    let _ = (TEXT,);
    runtime::reset(ResetType::WARM, Status::SUCCESS, None)
}
