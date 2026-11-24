//! The platform-neutral parts of anathema-setup: the FAT32 writer and the files that go on the
//! stick's EFI partition. The Windows GUI (src/app.rs) and the anathema-esp dev tool use them.

pub mod fat32;

/// The commpage map stubs anathema-xboot installs (built by tools/mkxcp.sh from
/// tools/commpage_stub.c; our own code, not Apple's).
pub const XCP: &[u8] = include_bytes!("../assets/xcp.bin");
