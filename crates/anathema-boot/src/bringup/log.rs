use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

const CAP: usize = 48 * 1024;
static mut BUF: [u8; CAP] = [0; CAP];
static mut LEN: usize = 0;
static LOCK: AtomicBool = AtomicBool::new(false);

struct Sink;

impl Write for Sink {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        unsafe {
            let buf = core::ptr::addr_of_mut!(BUF) as *mut u8;
            for byte in text.bytes() {
                if LEN < CAP {
                    *buf.add(LEN) = byte;
                    LEN += 1;
                }
            }
        }
        Ok(())
    }
}

fn lock() {
    while LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

pub fn line(args: fmt::Arguments) {
    lock();
    let _ = Sink.write_fmt(args);
    let _ = Sink.write_str("\r\n");
    unlock();
}

pub fn text() -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(BUF) as *const u8, LEN) }
}

pub struct Line {
    pub buf: [u8; 192],
    pub len: usize,
}

impl Line {
    pub const fn new() -> Line {
        Line { buf: [0; 192], len: 0 }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = byte;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! blog {
    ($($arg:tt)*) => {
        $crate::bringup::log::line(format_args!($($arg)*))
    };
}
