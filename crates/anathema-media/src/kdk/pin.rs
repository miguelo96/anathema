//! The Kernel Debug Kits anathema-xboot's kernel files come from, pinned by size and sha256
//! (GitHub's asset digests for dortania/KdkSupportPkg releases).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kdk {
    pub build: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// macOS 26.7.3 (Tahoe): the platform drivers.
pub const KDK_TAHOE: Kdk = Kdk {
    build: "25G342",
    url: "https://github.com/dortania/KdkSupportPkg/releases/download/25G342/Kernel_Debug_Kit_26.7.3_build_25G342.dmg",
    size: 1_181_028_172,
    sha256: "219f27c7e039b6ab3426496e0d5852d55a31d00b3d3984d855cba04377c5dba0",
};

/// macOS 27.2.1: the kernel, and corecrypto, Image4, pthread, Libm, Compression and friends.
pub const KDK_27: Kdk = Kdk {
    build: "26B131",
    url: "https://github.com/dortania/KdkSupportPkg/releases/download/26B131/Kernel_Debug_Kit_27.2.1_build_26B131.dmg",
    size: 1_251_401_698,
    sha256: "287377a4c52ae3ad04476a35195fc143129a18a2d34cd585cbec7b35cfc42c81",
};

pub const ALL: [Kdk; 2] = [KDK_TAHOE, KDK_27];

/// Where the kernel sits inside the KDK payload.
pub const KERNEL_PATH: &str = "System/Library/Kernels/kernel";
/// xnu-13432.41.4~25 RELEASE_X86_64: the kernel anathema-xboot's patch tables are written for.
pub const KERNEL_UUID: [u8; 16] = [0x03, 0x7E, 0x7B, 0x86, 0x0A, 0xC8, 0x3C, 0x67, 0xAF, 0xB8, 0x7C, 0x2C, 0x86, 0x43, 0x5E, 0x9A];
pub const KERNEL_SHA256: &str = "3c9317518535791503970fbec322b9bba70e91f3a233536205ccd77b047ffc83";

pub fn by_build(build: &str) -> Option<&'static Kdk> {
    ALL.iter().find(|k| k.build == build)
}

/// `037E7B86-0AC8-…` form of a Mach-O UUID.
pub fn uuid_string(u: &[u8; 16]) -> String {
    let h: Vec<String> = u.iter().map(|b| format!("{b:02X}")).collect();
    format!("{}-{}-{}-{}-{}", h[..4].concat(), h[4..6].concat(), h[6..8].concat(), h[8..10].concat(), h[10..].concat())
}
