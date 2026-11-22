#![cfg_attr(not(feature = "std"), no_std)]
#[macro_use]
extern crate alloc;

macro_rules! println {
    () => { $crate::sys::log("\n") };
    ($($t:tt)*) => {{
        $crate::sys::log(&alloc::format!($($t)*));
        $crate::sys::log("\n");
    }};
}

macro_rules! print {
    ($($t:tt)*) => {{
        $crate::sys::log(&alloc::format!($($t)*));
    }};
}

pub mod sys;
pub mod boot;
pub mod dt;
pub mod gic;
pub mod machine;
pub mod macho;
pub mod mem;
pub mod mmu;
pub mod pci;
pub mod simd;
pub mod vm;
