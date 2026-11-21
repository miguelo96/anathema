use super::{crash, log, prep, results, CORES, STATE_FAULT, STATE_READY};
use crate::gfx::{DIM, EMBER, EMBER_SOFT, LINE, RED, TEXT};
use core::sync::atomic::Ordering;

pub fn draw_status() {
    let Some(fb) = crash::fb() else {
        return;
    };
    let prep = prep();
    let res = results();
    fb.clear();
    let top = fb.header("bring-up test");
    let s = fb.scale();
    let x = 12 * s;
    let lh = 10 * s;
    let mut y = top + 4 * s;
    fb.spaced(x, y, "SYSTEM", s, 2 * s, EMBER);
    y += lh + 2 * s;
    fb.fill(x, y - s, fb.w - 2 * x, 1, LINE);
    let stats = res.paging.as_ref().unwrap();
    fb.print(x, y, s, TEXT, format_args!("tsc {} MHz (hpet check {} MHz)    lapic timer {} MHz at /16    local apic {} base {:#x} id {}", prep.tsc_hz / 1_000_000, res.hpet_tsc_hz / 1_000_000, res.lapic_hz / 1_000_000, if res.apic_x2 { "x2apic" } else { "xapic" }, res.apic_base, res.apic_id));
    y += lh;
    let deadline = match res.deadline {
        1 => "ok",
        2 => "failed",
        3 => "not supported",
        _ => "untested",
    };
    fb.print(x, y, s, TEXT, format_args!("int3 {}    timer irq {} ({} us for 1000 asked)    tsc-deadline {}    arat {}", if res.int3 { "ok" } else { "FAILED" }, if res.irq_ok { "ok" } else { "FAILED" }, res.irq_us, deadline, res.arat));
    y += lh;
    fb.print(x, y, s, TEXT, format_args!("paging cr3 {:#x}: {} x 2M ({} uc), {} x 1G ({} uc), ram top {:#x}", stats.cr3, stats.pages_2m, stats.pages_2m_uc, stats.pages_1g, stats.pages_1g_uc, stats.ram_top));
    y += lh;
    let total = prep.cores;
    let colour = if res.aps_ok + 1 == total { EMBER_SOFT } else { RED };
    fb.print(x, y, s, colour, format_args!("cores {} of {} up ({} listed in madt, trampoline {:#x})", res.aps_ok + 1, total, prep.listed, prep.tramp));
    y += lh + 6 * s;
    fb.spaced(x, y, "CORES", s, 2 * s, EMBER);
    y += lh + 2 * s;
    fb.fill(x, y - s, fb.w - 2 * x, 1, LINE);
    fb.print(x, y, s, DIM, format_args!("#   apic  state   fam  model step  tsc               wake us   same stack"));
    y += lh;
    let bsp = &CORES[0];
    for index in 0..total {
        let slot = &CORES[index];
        let ver = slot.ver.load(Ordering::SeqCst) as u32;
        let base = (ver >> 8) & 0xf;
        let family = if base == 0xf { base + ((ver >> 20) & 0xff) } else { base };
        let model = ((ver >> 4) & 0xf) | (((ver >> 16) & 0xf) << 4);
        let state = slot.state.load(Ordering::SeqCst);
        let (name, colour) = if state == STATE_READY {
            ("up", TEXT)
        } else if state == STATE_FAULT {
            ("fault", RED)
        } else if state == 0 {
            ("silent", RED)
        } else {
            ("stuck", RED)
        };
        let wake = if index == 0 {
            0
        } else {
            slot.tsc.load(Ordering::SeqCst).wrapping_sub(slot.launched.load(Ordering::SeqCst)) / (prep.tsc_hz / 1_000_000).max(1)
        };
        let same = slot.cr3.load(Ordering::SeqCst) == bsp.cr3.load(Ordering::SeqCst)
            && slot.cr4.load(Ordering::SeqCst) & !(1 << 17) == bsp.cr4.load(Ordering::SeqCst) & !(1 << 17)
            && slot.mtrr.load(Ordering::SeqCst) == bsp.mtrr.load(Ordering::SeqCst)
            && slot.pat.load(Ordering::SeqCst) == bsp.pat.load(Ordering::SeqCst);
        fb.print(
            x,
            y,
            s,
            colour,
            format_args!(
                "{:<3} {:<5} {:<7} {:#04x} {:#05x} {:<5} {:#018x} {:<9} {}  {:#x}",
                index,
                slot.apic_id.load(Ordering::SeqCst),
                name,
                family,
                model,
                ver & 0xf,
                slot.tsc.load(Ordering::SeqCst),
                wake,
                if same { "yes" } else { "no " },
                slot.rsp.load(Ordering::SeqCst)
            ),
        );
        y += lh;
    }
    y += 6 * s;
    fb.spaced(x, y, "LOG", s, 2 * s, EMBER);
    y += lh + 2 * s;
    fb.fill(x, y - s, fb.w - 2 * x, 1, LINE);
    let room = (fb.h.saturating_sub(y + 24 * s)) / lh;
    let text = log::text();
    let mut start = text.len();
    let mut seen = 0;
    while start > 0 && seen <= room {
        start -= 1;
        if text[start] == b'\n' {
            seen += 1;
        }
    }
    if start > 0 {
        start += 1;
    }
    let mut line = log::Line::new();
    for &byte in &text[start..] {
        if byte == b'\n' {
            fb.text(x, y, line.as_str(), s, DIM);
            y += lh;
            line.len = 0;
        } else if byte != b'\r' && line.len < line.buf.len() {
            line.buf[line.len] = byte;
            line.len += 1;
        }
    }
    fb.flush();
}
