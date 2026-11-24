use anathema_setup::fat32::FileSpec;
use anathema_setup::kernel_files::{self, esp_files, kc_config, prepare_with, space_check, update_plan, KernelFiles};
use std::path::{Path, PathBuf};

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("anathema-kf-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn space_check_reports_need_and_free() {
    let err = space_check(1_000_000_000, 3_600_000_000, Path::new("C:\\Users\\x\\AppData\\Local\\Anathema\\kdk")).unwrap_err();
    assert!(err.contains("3.6 GB needed, 1.0 GB free"), "{err}");
    assert!(err.contains("Anathema\\kdk"), "{err}");
    assert!(space_check(4_000_000_000, 3_600_000_000, Path::new("C:\\")).is_ok());
}

/// Writes a finished cache: out/xkernel, out/xkc.bin and a stamp recording their hashes.
fn seed_cache(cache: &Path, stamp: &str) {
    let out = cache.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("xkernel"), b"kernel bytes").unwrap();
    std::fs::write(out.join("xkc.bin"), b"kc bytes").unwrap();
    kernel_files::write_stamp(&out, stamp, b"kernel bytes", b"kc bytes").unwrap();
}

fn refusing_fetch() -> impl FnMut(&anathema_media::kdk::pin::Kdk, &Path, &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    |k, _, _| Err(format!("fetch called for {}", k.build))
}

#[test]
fn stamp_mismatch_forces_rebuild() {
    let cache = temp("stale");
    seed_cache(&cache, "an older setup");
    let err = prepare_with(&cache, u64::MAX, &mut refusing_fetch(), &mut |_, _| {}, &mut |_| {}).err().unwrap();
    assert_eq!(err, "fetch called for 25G342");
}

#[test]
fn stamp_match_uses_cache() {
    let cache = temp("fresh");
    seed_cache(&cache, &kernel_files::stamp());
    let k = prepare_with(&cache, u64::MAX, &mut refusing_fetch(), &mut |_, _| {}, &mut |_| {}).unwrap();
    assert_eq!((k.xkernel.as_slice(), k.xkc.as_slice()), (&b"kernel bytes"[..], &b"kc bytes"[..]));
}

#[test]
fn corrupt_cache_forces_rebuild() {
    let cache = temp("corrupt");
    seed_cache(&cache, &kernel_files::stamp());
    std::fs::write(cache.join("out/xkc.bin"), b"kc bytez").unwrap();
    assert!(prepare_with(&cache, u64::MAX, &mut refusing_fetch(), &mut |_, _| {}, &mut |_| {}).is_err());
}

#[test]
fn no_space_stops_before_any_download() {
    let cache = temp("nospace");
    let err = prepare_with(&cache, 1_000_000, &mut refusing_fetch(), &mut |_, _| {}, &mut |_| {}).err().unwrap();
    assert!(err.contains("Not enough free space"), "{err}");
}

#[test]
fn update_plan_rejects_full_esp() {
    let big = vec![0u8; 60 << 20];
    let files = [FileSpec { path: "anathema/xkc.bin", data: &big }];
    let err = update_plan(10 << 20, &[], &files).unwrap_err();
    assert!(err.contains("too small"), "{err}");
    // replacing a file of the same size still needs room for the new copy while both exist
    assert!(update_plan(10 << 20, &[("anathema/xkc.bin".into(), 60 << 20)], &files).is_err());
    assert!(update_plan(70 << 20, &[("anathema/xkc.bin".into(), 60 << 20)], &files).is_ok());
}

#[test]
fn update_plan_counts_space_freed_by_earlier_replacements() {
    let a = vec![0u8; 30 << 20];
    let b = vec![0u8; 30 << 20];
    let files = [FileSpec { path: "anathema/xkernel", data: &a }, FileSpec { path: "anathema/xkc.bin", data: &b }];
    // 35 MB free: xkernel's new copy fits, then its old 30 MB copy is freed before xkc.bin
    let existing = [("anathema/xkernel".to_string(), 30u64 << 20), ("anathema/xkc.bin".to_string(), 30 << 20)];
    assert!(update_plan(35 << 20, &existing, &files).is_ok());
    assert!(update_plan(35 << 20, &[], &files).is_err());
}

#[test]
fn esp_files_paths() {
    let k = KernelFiles { xkernel: vec![1], xkc: vec![2] };
    let paths: Vec<&str> = esp_files(b"efi", &k).iter().map(|f| f.path).collect();
    assert_eq!(paths, ["EFI/BOOT/BOOTX64.EFI", "anathema/xkernel", "anathema/xkc.bin", "anathema/xcp.bin"]);
    assert_eq!(esp_files(b"efi", &k)[3].data, anathema_setup::XCP);
}

#[test]
fn kc_config_matches_mkkc() {
    let c = kc_config(Path::new("T"), Path::new("S"), Path::new("K"));
    assert_eq!(c.ext, [PathBuf::from("T"), PathBuf::from("S")]);
    assert_eq!(c.kernel, PathBuf::from("K"));
    assert_eq!(c.base, 0xffff_ff80_0200_0000);
    assert_eq!(
        &c.roots[..9],
        [
            "com.apple.driver.AppleACPIPlatform", "com.apple.kec.corecrypto", "com.apple.kec.AppleEncryptedArchive",
            "com.apple.kec.XrtHostedXnu", "com.apple.kec.Libm", "com.apple.kec.Compression", "com.apple.kec.pthread",
            "com.apple.security.AppleImage4", "com.apple.kext.CoreTrust",
        ]
    );
    let file_roots: Vec<&str> = include_str!("../../../tools/kc-roots.txt").lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    assert_eq!(&c.roots[9..], file_roots.as_slice());
    assert_eq!(
        c.prefer_last,
        [
            "com.apple.kec.corecrypto", "com.apple.kec.Compression", "com.apple.kec.pthread", "com.apple.kec.Libm",
            "com.apple.kec.AppleEncryptedArchive", "com.apple.kec.XrtHostedXnu", "com.apple.security.AppleImage4",
        ]
    );
    assert_eq!(
        c.strip,
        [
            ("com.apple.security.AppleImage4".to_string(), "AppleSecurityExtension".to_string()),
            ("com.apple.kext.CoreTrust".to_string(), "AppleSecurityExtension".to_string()),
        ]
    );
    assert_eq!(c.codeless, ["com.apple.driver.AppleMobileFileIntegrity"]);
    assert!(c.no_start.is_empty() && c.set_true.is_empty() && c.rename.is_empty() && c.add_lib.is_empty() && c.overrides.is_empty());
}
