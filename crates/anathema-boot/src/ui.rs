use crate::gfx::*;
use crate::input::{self, Key};
use crate::report::{Report, Tone, STEPS};
use alloc::format;
use alloc::string::String;

pub const NAME: &str = "ANATHEMA";
pub const VERSION: &str = "0.1";

pub fn header(screen: &mut Screen, subtitle: &str) -> usize {
    let s = screen.scale();
    let side = 24 * s;
    let gap = 8 * s;
    let name_w = Screen::spaced_width(NAME, s * 2, 6 * s);
    let total = side + gap * 2 + name_w;
    let left = screen.w.saturating_sub(total) / 2;
    let top = 14 * s;
    screen.sigil(left, top, side);
    let name_y = top + (side - 16 * s) / 2;
    screen.spaced(left + side + gap * 2, name_y, NAME, s * 2, 6 * s, TEXT);
    let rule_y = top + side + 8 * s;
    screen.fill(screen.w / 2 - 20 * s, rule_y, 40 * s, s.max(2) / 2 + 1, EMBER);
    screen.text_centred(rule_y + 6 * s, subtitle, s, DIM);
    rule_y + 22 * s
}

pub fn footer(screen: &mut Screen, keys: &[(&str, &str)], note: &str) {
    let s = screen.scale();
    let y = screen.h.saturating_sub(18 * s);
    let mut width = 0;
    for (key, label) in keys {
        width += (key.len() + label.len() + 3) * 8 * s;
    }
    let mut x = screen.w.saturating_sub(width) / 2;
    for (key, label) in keys {
        screen.text(x, y, key, s, EMBER);
        x += (key.len() + 1) * 8 * s;
        screen.text(x, y, label, s, DIM);
        x += (label.len() + 2) * 8 * s;
    }
    if !note.is_empty() {
        screen.text_centred(y - 12 * s, note, s, EMBER_SOFT);
    }
}

pub fn splash(screen: &mut Screen, done: usize, subtitle: &str) {
    screen.clear();
    let s = screen.scale();
    let top = header(screen, subtitle);
    let col = 220 * s;
    let left = screen.w.saturating_sub(col) / 2;
    let mut y = top + 20 * s;
    for (index, name) in STEPS.iter().enumerate() {
        let (mark, colour) = if index < done {
            ("ok ", DIM)
        } else if index == done {
            (">  ", EMBER)
        } else {
            ("   ", LINE)
        };
        screen.text(left, y, mark, s, colour);
        screen.text(left + 24 * s, y, name, s, if index == done { TEXT } else { colour });
        y += 12 * s;
    }
    let bar_y = y + 12 * s;
    screen.fill(left, bar_y, col, 3 * s, RAISED);
    screen.fill(left, bar_y, col * done / STEPS.len(), 3 * s, EMBER);
    screen.present();
}

pub struct Entry {
    pub label: &'static str,
    pub hint: &'static str,
    pub enabled: bool,
}

pub fn menu(screen: &mut Screen, entries: &[Entry], subtitle: &str, note: &str, selected: usize) {
    screen.clear();
    let s = screen.scale();
    let top = header(screen, subtitle);
    let col = (300 * s).min(screen.w - 32);
    let left = screen.w.saturating_sub(col) / 2;
    let row = 30 * s;
    let mut y = top + 18 * s;
    for (index, entry) in entries.iter().enumerate() {
        let chosen = index == selected;
        if chosen {
            screen.fill(left, y, col, row - 4 * s, RAISED);
            screen.fill(left, y, 3 * s, row - 4 * s, EMBER);
        } else {
            screen.fill(left, y, col, row - 4 * s, PANEL);
        }
        let colour = if !entry.enabled {
            LINE
        } else if chosen {
            TEXT
        } else {
            DIM
        };
        screen.text(left + 14 * s, y + 5 * s, entry.label, s, colour);
        let hint_colour = if chosen && entry.enabled { EMBER_SOFT } else { LINE };
        screen.text(left + 14 * s, y + 5 * s + 10 * s, entry.hint, s, hint_colour);
        y += row;
    }
    footer(
        screen,
        &[("up/down", "choose"), ("enter", "select")],
        note,
    );
    screen.present();
}

pub fn viewer(screen: &mut Screen, report: &Report, saved: &str) {
    let s = (screen.h / 540).max(1);
    let mut first = 0usize;
    let mut note = String::from(saved);
    loop {
        screen.clear();
        let side = 14 * s;
        screen.sigil(12 * s, 8 * s, side);
        screen.spaced(12 * s + side + 8 * s, 8 * s + (side - 8 * s) / 2, "HARDWARE REPORT", s, 3 * s, EMBER);
        screen.fill(12 * s, 8 * s + side + 5 * s, screen.w - 24 * s, 1, LINE);
        let body_top = 8 * s + side + 12 * s;
        let body_bottom = screen.h.saturating_sub(34 * s);
        let line_h = 10 * s;
        let rows = (body_bottom.saturating_sub(body_top)) / line_h;
        let cols = (screen.w.saturating_sub(24 * s)) / (8 * s);
        let max_first = report.lines.len().saturating_sub(rows);
        first = first.min(max_first);
        for (i, line) in report.lines.iter().skip(first).take(rows).enumerate() {
            let colour = match line.tone {
                Tone::Head => EMBER,
                Tone::Normal => TEXT,
                Tone::Dim => DIM,
                Tone::Warn => EMBER_SOFT,
                Tone::Bad => RED,
            };
            let mut text = line.text.as_str();
            if text.len() > cols {
                text = &text[..cols];
            }
            screen.text(12 * s, body_top + i * line_h, text, s, colour);
        }
        let position = format!("{}-{} of {}", first + 1, (first + rows).min(report.lines.len()), report.lines.len());
        screen.text(screen.w.saturating_sub(position.len() * 8 * s + 12 * s), 8 * s + (side - 8 * s) / 2, &position, s, DIM);
        footer(
            screen,
            &[("up/down", "scroll"), ("pgup/pgdn", "page"), ("s", "save log"), ("esc", "back")],
            &note,
        );
        screen.present();
        match input::next() {
            Key::Up => first = first.saturating_sub(1),
            Key::Down => first = (first + 1).min(max_first),
            Key::PageUp => first = first.saturating_sub(rows.saturating_sub(2)),
            Key::PageDown => first = (first + rows.saturating_sub(2)).min(max_first),
            Key::Home => first = 0,
            Key::End => first = max_first,
            Key::Esc => return,
            Key::Char('s') | Key::Char('S') => {
                note = crate::store::save(&report.plain());
            }
            _ => {}
        }
    }
}
