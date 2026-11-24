//! Kernel Debug Kit unpacking: the dmg (UDIF) holds an HFS+ volume with a xar package whose
//! payload is a pbzx stream of xz chunks around an odc cpio archive.

pub mod cpio;
pub mod pbzx;
