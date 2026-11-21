use crate::gfx::{DIM, EMBER, EMBER_DEEP, EMBER_SOFT, TEXT};
use core::fmt::Write;
use font8x8::legacy::BASIC_LEGACY;

pub const BG: u32 = 0x0b0a0a;

pub struct Fb {
    pub base: *mut u8,
    pub stride: usize,
    pub w: usize,
    pub h: usize,
    pub bgr: bool,
    pub shadow: *mut u32,
}

unsafe impl Send for Fb {}
unsafe impl Sync for Fb {}

impl Fb {
    pub fn scale(&self) -> usize {
        (self.h / 540).max(1)
    }

    pub fn clear(&mut self) {
        self.fill(0, 0, self.w, self.h, BG);
    }

    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u32) {
        let right = (x + w).min(self.w);
        for row in y..(y + h).min(self.h) {
            for col in x..right {
                unsafe { *self.shadow.add(row * self.w + col) = colour };
            }
        }
    }

    pub fn glyph(&mut self, x: usize, y: usize, ch: u8, scale: usize, colour: u32) {
        let code = if ch < 128 { ch as usize } else { b'?' as usize };
        let rows = BASIC_LEGACY[code];
        for (gy, bits) in rows.iter().enumerate() {
            for gx in 0..8 {
                if (bits >> gx) & 1 == 1 {
                    self.fill(x + gx * scale, y + gy * scale, scale, scale, colour);
                }
            }
        }
    }

    pub fn text(&mut self, x: usize, y: usize, text: &str, scale: usize, colour: u32) {
        for (i, ch) in text.bytes().enumerate() {
            let at = x + i * 8 * scale;
            if at + 8 * scale > self.w {
                break;
            }
            self.glyph(at, y, ch, scale, colour);
        }
    }

    pub fn spaced(&mut self, x: usize, y: usize, text: &str, scale: usize, gap: usize, colour: u32) {
        for (i, ch) in text.bytes().enumerate() {
            self.glyph(x + i * (8 * scale + gap), y, ch, scale, colour);
        }
    }

    pub fn print(&mut self, x: usize, y: usize, scale: usize, colour: u32, args: core::fmt::Arguments) {
        let mut line = super::log::Line::new();
        let _ = line.write_fmt(args);
        self.text(x, y, line.as_str(), scale, colour);
    }

    pub fn ring(&mut self, cx: usize, cy: usize, radius: usize) {
        let outer = (radius * radius) as i64;
        let inner_r = radius * 70 / 100;
        let inner = (inner_r * inner_r) as i64;
        let core_r = radius * 17 / 100;
        let core = (core_r * core_r) as i64;
        for py in 0..radius * 2 {
            for px in 0..radius * 2 {
                let dx = px as i64 - radius as i64;
                let dy = py as i64 - radius as i64;
                let d2 = dx * dx + dy * dy;
                let in_ring = d2 < outer && d2 >= inner;
                let in_core = d2 < core;
                let gap = in_ring && dx > 0 && dy < 0 && (dx + dy).abs() < (radius as i64 * 16 / 100);
                if (in_ring && !gap) || in_core {
                    let colour = if py * 2 < radius * 2 { EMBER_SOFT } else { EMBER_DEEP };
                    if cx + px >= radius && cy + py >= radius {
                        self.fill(cx + px - radius, cy + py - radius, 1, 1, colour);
                    }
                }
            }
        }
    }

    pub fn header(&mut self, subtitle: &str) -> usize {
        let s = self.scale();
        let side = 12 * s;
        let name = "ANATHEMA";
        let name_w = name.len() * (16 * s + 6 * s) - 6 * s;
        let total = side * 2 + 12 * s + name_w;
        let left = self.w.saturating_sub(total) / 2;
        let top = 10 * s;
        self.ring(left + side, top + side, side);
        self.spaced(left + side * 2 + 12 * s, top + side - 8 * s, name, s * 2, 6 * s, TEXT);
        let rule = top + side * 2 + 6 * s;
        self.fill(self.w / 2 - 20 * s, rule, 40 * s, 2, EMBER);
        let sub_w = subtitle.len() * 8 * s;
        self.text(self.w.saturating_sub(sub_w) / 2, rule + 6 * s, subtitle, s, DIM);
        rule + 22 * s
    }

    pub fn flush(&mut self) {
        self.flush_rows(0, self.h);
    }

    pub fn flush_rows(&mut self, first: usize, last: usize) {
        for row in first..last.min(self.h) {
            unsafe {
                let src = self.shadow.add(row * self.w);
                let dst = self.base.add(row * self.stride * 4) as *mut u32;
                if self.bgr {
                    core::ptr::copy_nonoverlapping(src, dst, self.w);
                } else {
                    for col in 0..self.w {
                        let p = *src.add(col);
                        let swapped = (p & 0x00ff00) | ((p & 0xff0000) >> 16) | ((p & 0xff) << 16);
                        core::ptr::write_volatile(dst.add(col), swapped);
                    }
                }
            }
        }
    }
}
