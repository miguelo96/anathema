use alloc::vec;
use alloc::vec::Vec;
use font8x8::legacy::BASIC_LEGACY;
use uefi::boot::{self, ScopedProtocol};
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput, PixelFormat};

pub const BACKGROUND: u32 = 0x0b0a0a;
pub const PANEL: u32 = 0x151313;
pub const RAISED: u32 = 0x211c1a;
pub const LINE: u32 = 0x3a302c;
pub const TEXT: u32 = 0xede7e2;
pub const DIM: u32 = 0x9a8f89;
pub const EMBER: u32 = 0xff6a2b;
pub const EMBER_SOFT: u32 = 0xffa05a;
pub const EMBER_DEEP: u32 = 0xd2321a;
pub const RED: u32 = 0xff4a4a;

#[derive(Clone)]
pub struct GopInfo {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub format: &'static str,
    pub base: usize,
    pub size: usize,
    pub modes: Vec<(usize, usize)>,
    pub current_mode: usize,
}

pub struct Screen {
    gop: ScopedProtocol<GraphicsOutput>,
    pub w: usize,
    pub h: usize,
    buf: Vec<BltPixel>,
    pub info: GopInfo,
}

fn pixel(colour: u32) -> BltPixel {
    BltPixel::new((colour >> 16) as u8, (colour >> 8) as u8, colour as u8)
}

fn channels(colour: u32) -> [u32; 3] {
    [(colour >> 16) & 255, (colour >> 8) & 255, colour & 255]
}

pub fn mix(from: u32, to: u32, num: u32, den: u32) -> u32 {
    let a = channels(from);
    let b = channels(to);
    let mut out = 0;
    for i in 0..3 {
        let v = (a[i] * (den - num) + b[i] * num + den / 2) / den;
        out = (out << 8) | v;
    }
    out
}

impl Screen {
    pub fn open() -> Option<Screen> {
        let handle = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
        let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle).ok()?;
        let mode = gop.current_mode_info();
        let (w, h) = mode.resolution();
        let format = match mode.pixel_format() {
            PixelFormat::Rgb => "rgb",
            PixelFormat::Bgr => "bgr",
            PixelFormat::Bitmask => "bitmask",
            PixelFormat::BltOnly => "blt only",
        };
        let stride = mode.stride();
        let mut modes = Vec::new();
        let mut current_mode = 0;
        for (index, candidate) in gop.modes().enumerate() {
            let res = candidate.info().resolution();
            if res == (w, h) && current_mode == 0 {
                current_mode = index;
            }
            modes.push(res);
        }
        let mut frame = gop.frame_buffer();
        let base = frame.as_mut_ptr() as usize;
        let size = frame.size();
        Some(Screen {
            gop,
            w,
            h,
            buf: vec![pixel(BACKGROUND); w * h],
            info: GopInfo {
                width: w,
                height: h,
                stride,
                format,
                base,
                size,
                modes,
                current_mode,
            },
        })
    }

    pub fn scale(&self) -> usize {
        (self.h / 360).max(1)
    }

    pub fn clear(&mut self) {
        self.buf.fill(pixel(BACKGROUND));
    }

    pub fn present(&mut self) {
        let _ = self.gop.blt(BltOp::BufferToVideo {
            buffer: &self.buf,
            src: BltRegion::Full,
            dest: (0, 0),
            dims: (self.w, self.h),
        });
    }

    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u32) {
        let p = pixel(colour);
        let right = (x + w).min(self.w);
        for row in y..(y + h).min(self.h) {
            if x < right {
                self.buf[row * self.w + x..row * self.w + right].fill(p);
            }
        }
    }


    fn blend(&mut self, x: usize, y: usize, colour: u32, num: u32, den: u32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let at = y * self.w + x;
        let p = self.buf[at];
        let under = ((p.red as u32) << 16) | ((p.green as u32) << 8) | p.blue as u32;
        self.buf[at] = pixel(mix(under, colour, num, den));
    }

    pub fn glyph(&mut self, x: usize, y: usize, ch: char, scale: usize, colour: u32) {
        let code = if (ch as usize) < 128 { ch as usize } else { b'?' as usize };
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
        for (i, ch) in text.chars().enumerate() {
            self.glyph(x + i * 8 * scale, y, ch, scale, colour);
        }
    }

    pub fn spaced(&mut self, x: usize, y: usize, text: &str, scale: usize, gap: usize, colour: u32) {
        for (i, ch) in text.chars().enumerate() {
            self.glyph(x + i * (8 * scale + gap), y, ch, scale, colour);
        }
    }

    pub fn spaced_width(text: &str, scale: usize, gap: usize) -> usize {
        let n = text.chars().count();
        (n * (8 * scale + gap)).saturating_sub(gap)
    }

    pub fn text_centred(&mut self, y: usize, text: &str, scale: usize, colour: u32) {
        let width = text.chars().count() * 8 * scale;
        self.text(self.w.saturating_sub(width) / 2, y, text, scale, colour);
    }

    pub fn sigil(&mut self, x: usize, y: usize, side: usize) {
        const N: usize = 3;
        for py in 0..side {
            for px in 0..side {
                let mut hit = 0u32;
                for sy in 0..N {
                    for sx in 0..N {
                        let u = (px * N + sx) as f32 / (side * N) as f32;
                        let v = (py * N + sy) as f32 / (side * N) as f32;
                        if inside(u, v) {
                            hit += 1;
                        }
                    }
                }
                if hit > 0 {
                    let colour = mix(EMBER_SOFT, EMBER_DEEP, py as u32, side as u32);
                    self.blend(x + px, y + py, colour, hit, (N * N) as u32);
                }
            }
        }
    }
}

fn inside(u: f32, v: f32) -> bool {
    let dx = u - 0.5;
    let dy = v - 0.5;
    let d2 = dx * dx + dy * dy;
    let ring = d2 >= 0.31 * 0.31 && d2 < 0.46 * 0.46;
    let core = d2 < 0.075 * 0.075;
    if ring && dx > 0.0 && dy < 0.0 {
        let sum = dx + dy;
        if sum < 0.075 && sum > -0.075 {
            return false;
        }
    }
    ring || core
}
