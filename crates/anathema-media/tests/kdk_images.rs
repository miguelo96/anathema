//! dmg, HFS+ and xar readers. On macOS the fixtures are made when the tests run, with
//! `pkgbuild` and `hdiutil`; nothing Apple-made is stored in the repository.

use anathema_media::kdk::{hfs::Hfs, udif::Udif, xar::Xar};
use anathema_media::readat::{FileAt, ReadAt};
use std::sync::Arc;

fn arc(r: impl ReadAt + Send + Sync + 'static) -> Arc<dyn ReadAt + Send + Sync> {
    Arc::new(r)
}

#[test]
fn udif_rejects_bzip2_chunk() {
    // one partition with a single bzip2 chunk, then the terminator
    let mut mish = vec![0u8; 204];
    mish[..4].copy_from_slice(b"mish");
    mish[16..24].copy_from_slice(&1u64.to_be_bytes());
    mish[200..204].copy_from_slice(&2u32.to_be_bytes());
    let chunk = |kind: u32, sectors: u64| {
        let mut c = vec![0u8; 40];
        c[..4].copy_from_slice(&kind.to_be_bytes());
        c[16..24].copy_from_slice(&sectors.to_be_bytes());
        c[32..40].copy_from_slice(&16u64.to_be_bytes());
        c
    };
    mish.extend(chunk(0x8000_0006, 1));
    mish.extend(chunk(0xffff_ffff, 0));
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&mish);
    let xml = format!(
        "<plist><dict><key>resource-fork</key><dict><key>blkx</key><array><dict><key>Name</key><string>disk image (Apple_HFS : 1)</string><key>Data</key><data>{b64}</data></dict></array></dict></dict></plist>"
    );
    let mut file = vec![0u8; 16];
    let xml_off = file.len() as u64;
    file.extend_from_slice(xml.as_bytes());
    let mut koly = vec![0u8; 512];
    koly[..4].copy_from_slice(b"koly");
    koly[0xd8..0xe0].copy_from_slice(&xml_off.to_be_bytes());
    koly[0xe0..0xe8].copy_from_slice(&(xml.len() as u64).to_be_bytes());
    file.extend(koly);
    let u = Udif::open(arc(file)).unwrap();
    assert_eq!(u.partitions()[0].sectors, 1);
    let p = u.open_partition(u.find_hfs().unwrap()).unwrap();
    let mut buf = [0u8; 16];
    let err = p.read_exact_at(0, &mut buf).unwrap_err().to_string();
    assert!(err.contains("dmg chunk type 0x80000006 is not supported"), "{err}");
}

#[test]
fn udif_rejects_non_dmg() {
    assert!(Udif::open(arc(vec![0u8; 4096])).is_err());
}

#[test]
fn xar_rejects_bad_magic() {
    assert!(Xar::open(arc(b"notxar-at-all-but-long-enough-for-a-header".to_vec())).is_err());
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::OnceLock;

    fn run(cmd: &mut Command) {
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn big() -> Vec<u8> {
        let mut x = 7u32;
        (0..3_000_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    /// (dir, [ULFO, UDZO, UDRO] dmg paths), made once per test run.
    fn fixtures() -> &'static (PathBuf, Vec<PathBuf>) {
        static F: OnceLock<(PathBuf, Vec<PathBuf>)> = OnceLock::new();
        F.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("anathema-kdk-images-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let (root, src) = (dir.join("pkgroot"), dir.join("src"));
            std::fs::create_dir_all(root.join("System/Library/Extensions/Test.kext/Contents")).unwrap();
            std::fs::write(root.join("a.txt"), vec![b'a'; 10_000]).unwrap();
            std::fs::write(root.join("System/Library/Extensions/Test.kext/Contents/Info.plist"), b"<plist/>").unwrap();
            std::fs::create_dir_all(src.join("pkg")).unwrap();
            std::fs::write(src.join("big.bin"), big()).unwrap();
            run(Command::new("pkgbuild").args(["--quiet", "--root"]).arg(&root).args(["--identifier", "test.kdk", "--install-location", "/"]).arg(src.join("pkg/test.pkg")));
            // one volume, three encodings: convert keeps the partition bytes identical
            let ulfo = dir.join("ULFO.dmg");
            run(Command::new("hdiutil").args(["create", "-quiet", "-fs", "HFSX", "-volname", "T", "-format", "ULFO", "-srcfolder"]).arg(&src).arg(&ulfo));
            let mut dmgs = vec![ulfo.clone()];
            for fmt in ["UDZO", "UDRO"] {
                let p = dir.join(format!("{fmt}.dmg"));
                run(Command::new("hdiutil").args(["convert", "-quiet"]).arg(&ulfo).args(["-format", fmt, "-o"]).arg(&p));
                dmgs.push(p);
            }
            (dir, dmgs)
        })
    }

    fn hfs_of(dmg: &Path) -> Hfs {
        let u = Udif::open(arc(FileAt::open(dmg).unwrap())).unwrap();
        let part = u.open_partition(u.find_hfs().unwrap()).unwrap();
        Hfs::open(arc(part)).unwrap()
    }

    #[test]
    fn udif_lzfse_zlib_raw_partitions_read_identically() {
        let (_, dmgs) = fixtures();
        let parts: Vec<_> = dmgs
            .iter()
            .map(|d| {
                let u = Udif::open(arc(FileAt::open(d).unwrap())).unwrap();
                u.open_partition(u.find_hfs().unwrap()).unwrap()
            })
            .collect();
        let len = parts[0].len();
        assert!(parts.iter().all(|p| p.len() == len));
        for off in [0u64, 1024, len / 3 + 17, len - 4096] {
            let want = parts[0].read_vec(off, 4096).unwrap();
            for p in &parts[1..] {
                assert!(p.read_vec(off, 4096).unwrap() == want, "offset {off}");
            }
        }
        assert_eq!(&parts[0].read_vec(1024, 2).unwrap(), b"HX");
    }

    #[test]
    fn hfs_reads_big_file_exactly() {
        let (_, dmgs) = fixtures();
        for d in dmgs {
            let f = hfs_of(d).open_file("big.bin").unwrap();
            assert!(f.read_vec(0, f.len() as usize).unwrap() == big(), "{}", d.display());
        }
        assert!(hfs_of(&dmgs[0]).open_file("missing.bin").is_err());
    }

    #[test]
    fn xar_finds_payload_in_pkg() {
        let (_, dmgs) = fixtures();
        let pkg = hfs_of(&dmgs[0]).open_file("pkg/test.pkg").unwrap();
        let x = Xar::open(arc(pkg)).unwrap();
        // pkgbuild makes a component package with Payload at the root; the KDK is a product
        // archive and its payload is at KDK.pkg/Payload
        let f = x.file("Payload").unwrap();
        assert_eq!(f.encoding, "application/octet-stream");
        let bom = x.file("Bom").unwrap();
        let err = x.reader(&bom).err().expect("gzip encoding is refused").to_string();
        assert!(err.contains("application/x-gzip"), "{err}");
        assert!(x.file("KDK.pkg/Payload").is_err());
    }

    #[test]
    fn pkg_payload_chain() {
        let (_, dmgs) = fixtures();
        let pkg = hfs_of(&dmgs[1]).open_file("pkg/test.pkg").unwrap();
        let x = Xar::open(arc(pkg)).unwrap();
        let payload = x.reader(&x.file("Payload").unwrap()).unwrap();
        let mut names = Vec::new();
        let mut a = Vec::new();
        anathema_media::kdk::cpio::for_each(flate2::read::GzDecoder::new(payload), |e, r| {
            if e.name == "a.txt" {
                r.read_to_end(&mut a)?;
            }
            names.push(e.name.clone());
            Ok(())
        })
        .unwrap();
        assert!(names.contains(&"a.txt".to_string()), "{names:?}");
        assert!(names.contains(&"System/Library/Extensions/Test.kext/Contents/Info.plist".to_string()), "{names:?}");
        assert_eq!(a, vec![b'a'; 10_000]);
    }
}
