//! Kext collection linker for anathema-xboot: a port of `tools/kcbuild.py`.

pub mod bundles;
pub mod link;
pub mod macho;
pub mod plistxml;

pub use link::{link, Config, Output, Report};
