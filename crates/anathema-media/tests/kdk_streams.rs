use anathema_media::kdk::{cpio, pbzx::Pbzx};
use std::io::{Cursor, Read};

fn odc(name: &str, mode: u32, data: &[u8]) -> Vec<u8> {
    let full = format!("{name}\0");
    let mut h = format!(
        "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
        1, 2, mode, 0, 0, 1, 0, 0, full.len(), data.len()
    )
    .into_bytes();
    assert_eq!(h.len(), 76);
    h.extend_from_slice(full.as_bytes());
    h.extend_from_slice(data);
    h
}

fn archive(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, m, d) in entries {
        out.extend(odc(n, *m, d));
    }
    out.extend(odc("TRAILER!!!", 0, b""));
    out
}

#[test]
fn cpio_reads_files_dirs_and_trailer() {
    let a = archive(&[
        ("./System", 0o040755, b""),
        ("./System/empty", 0o100644, b""),
        ("./System/three", 0o100644, b"abc"),
        ("./System/link", 0o120755, b"three"),
    ]);
    let mut seen = Vec::new();
    let n = cpio::for_each(Cursor::new(a), |e, r| {
        let mut body = Vec::new();
        if e.name.ends_with("three") {
            r.read_to_end(&mut body)?;
        } else if e.name.ends_with("link") {
            // read only part: the reader must skip the rest
            let mut one = [0u8; 1];
            r.read_exact(&mut one)?;
            body.extend_from_slice(&one);
        }
        seen.push((e.name.clone(), e.mode, e.size, body));
        Ok(())
    })
    .unwrap();
    assert_eq!(n, 4);
    assert_eq!(
        seen,
        vec![
            ("System".to_string(), 0o040755, 0, vec![]),
            ("System/empty".to_string(), 0o100644, 0, vec![]),
            ("System/three".to_string(), 0o100644, 3, b"abc".to_vec()),
            ("System/link".to_string(), 0o120755, 5, b"t".to_vec()),
        ]
    );
}

#[test]
fn cpio_rejects_bad_magic() {
    let mut a = archive(&[("./x", 0o100644, b"1")]);
    a[0] = b'1';
    let err = cpio::for_each(Cursor::new(a), |_, _| Ok(())).unwrap_err().to_string();
    assert!(err.contains("cpio"), "{err}");
}

#[test]
fn cpio_truncated_body_errors() {
    let mut a = archive(&[("./x", 0o100644, b"12345")]);
    a.truncate(76 + 4 + 2);
    assert!(cpio::for_each(Cursor::new(a), |_, _| Ok(())).is_err());
}

fn xz(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    lzma_rs::xz_compress(&mut &data[..], &mut out).unwrap();
    out
}

fn pbzx_file(chunks: &[(&[u8], bool)]) -> Vec<u8> {
    let mut f = b"pbzx".to_vec();
    f.extend_from_slice(&(16u64 << 20).to_be_bytes());
    for (data, compress) in chunks {
        let body = if *compress { xz(data) } else { data.to_vec() };
        f.extend_from_slice(&(data.len() as u64).to_be_bytes());
        f.extend_from_slice(&(body.len() as u64).to_be_bytes());
        f.extend_from_slice(&body);
    }
    f
}

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|i| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if i % 7 == 0 { (x >> 16) as u8 } else { b'a' + (i % 13) as u8 }
        })
        .collect()
}

#[test]
fn pbzx_round_trips_mixed_chunks() {
    let (a, b, c) = (noise(300_000, 1), noise(1000, 2), noise(250_000, 3));
    let file = pbzx_file(&[(&a, true), (&b, false), (&c, true)]);
    let want: Vec<u8> = [a, b, c].concat();
    for threads in [1, 4] {
        let mut p = Pbzx::new(Cursor::new(file.clone()), threads).unwrap();
        let consumed = p.consumed();
        let mut got = Vec::new();
        p.read_to_end(&mut got).unwrap();
        assert!(got == want, "threads={threads}: {} bytes vs {}", got.len(), want.len());
        assert_eq!(consumed.load(std::sync::atomic::Ordering::Relaxed), file.len() as u64);
    }
}

#[test]
fn pbzx_rejects_bad_magic() {
    let mut f = pbzx_file(&[(b"abc", false)]);
    f[0] = b'x';
    assert!(Pbzx::new(Cursor::new(f), 2).is_err());
}

#[test]
fn pbzx_truncated_chunk_errors() {
    let mut f = pbzx_file(&[(&noise(10_000, 5), true)]);
    f.truncate(f.len() - 10);
    let mut out = Vec::new();
    assert!(Pbzx::new(Cursor::new(f), 2).unwrap().read_to_end(&mut out).is_err());
}

#[test]
fn pbzx_rejects_chunk_that_is_not_xz() {
    let mut f = b"pbzx".to_vec();
    f.extend_from_slice(&(16u64 << 20).to_be_bytes());
    f.extend_from_slice(&100u64.to_be_bytes());
    f.extend_from_slice(&5u64.to_be_bytes());
    f.extend_from_slice(b"hello");
    let mut out = Vec::new();
    let err = Pbzx::new(Cursor::new(f), 1).unwrap().read_to_end(&mut out).unwrap_err().to_string();
    assert!(err.contains("xz"), "{err}");
}

/// Decodes a real pbzx payload when PBZX_FILE (and PBZX_SIZE, its decoded size) are set.
#[test]
#[ignore]
fn pbzx_real_payload() {
    let (Ok(path), Ok(size)) = (std::env::var("PBZX_FILE"), std::env::var("PBZX_SIZE")) else { return };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let t = std::time::Instant::now();
    let mut p = Pbzx::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()), threads).unwrap();
    let n = std::io::copy(&mut p, &mut std::io::sink()).unwrap();
    eprintln!("decoded {n} bytes in {:.1}s with {threads} threads", t.elapsed().as_secs_f64());
    assert_eq!(n, size.parse::<u64>().unwrap());
}

mod task7 {
    use anathema_media::download::verify_or_discard;
    use anathema_media::kdk::extract::{extract, wanted, MARKER};
    use anathema_media::kdk::pin;
    use std::path::{Path, PathBuf};

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anathema-kdk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn verify_or_discard_rejects_corrupt_file() {
        let f = temp("corrupt").join("x.dmg");
        std::fs::write(&f, b"not what was pinned").unwrap();
        assert!(!verify_or_discard(&f, pin::KDK_27.sha256).unwrap());
        assert!(!f.exists(), "a file that fails its hash is deleted");
    }

    #[test]
    fn verify_or_discard_accepts_good_file() {
        let f = temp("good").join("x.dmg");
        std::fs::write(&f, b"abc").unwrap();
        // sha256("abc")
        assert!(verify_or_discard(&f, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap());
        assert!(f.exists());
    }

    #[test]
    fn pins_match_the_design() {
        assert_eq!(pin::KDK_TAHOE.build, "25G342");
        assert_eq!(pin::KDK_TAHOE.size, 1_181_028_172);
        assert_eq!(pin::KDK_27.build, "26B131");
        assert_eq!(pin::KDK_27.url, "https://github.com/dortania/KdkSupportPkg/releases/download/26B131/Kernel_Debug_Kit_27.2.1_build_26B131.dmg");
        assert_eq!(pin::KDK_27.sha256, "287377a4c52ae3ad04476a35195fc143129a18a2d34cd585cbec7b35cfc42c81");
        assert_eq!(pin::KERNEL_SHA256, "3c9317518535791503970fbec322b9bba70e91f3a233536205ccd77b047ffc83");
        assert_eq!(pin::uuid_string(&pin::KERNEL_UUID), "037E7B86-0AC8-3C67-AFB8-7C2C86435E9A");
        assert_eq!(pin::by_build("26B131").map(|k| k.size), Some(1_251_401_698));
        assert!(pin::by_build("26B123").is_none());
    }

    #[test]
    fn wanted_filter_cases() {
        let yes = [
            "System/Library/Kernels/kernel",
            "System/Library/Extensions/A.kext/Contents/Info.plist",
            "System/Library/Extensions/A.kext/Contents/PlugIns/B.kext/Contents/MacOS/B",
        ];
        let no = [
            "System/Library/Kernels/kernel.development",
            "System/Library/Extensions/A.kext.dSYM/Contents/Info.plist",
            "System/Library/Extensions/README",
            "System/Library/Extensions/A.kext/../../../../etc/passwd",
            "System/Library/Extensions/A.kext/./x",
            "System/Library/Extensions/A.kext//x",
            "System/Library/Extensions/A.kext/Contents\\..\\..\\x",
            "Library/Developer/A.kext/Contents/Info.plist",
        ];
        for n in yes {
            assert!(wanted(n), "{n} should be extracted");
        }
        for n in no {
            assert!(!wanted(n), "{n} should be skipped");
        }
    }

    #[test]
    fn extract_refuses_a_non_empty_folder() {
        let d = temp("nonempty");
        std::fs::write(d.join("keep.txt"), b"mine").unwrap();
        let err = extract(Path::new("/nonexistent.dmg"), &d, &mut |_, _| {}).unwrap_err().to_string();
        assert!(err.contains("not empty"), "{err}");
        assert!(d.join("keep.txt").exists());
    }

    /// Fetches a 6 KB asset from the same GitHub release as the 26B131 KDK, from scratch and
    /// again after cutting the file short (resume). Needs the network.
    #[test]
    #[ignore]
    fn download_verified_from_github() {
        use anathema_media::download::download_verified;
        let url = "https://github.com/dortania/KdkSupportPkg/releases/download/26B131/kernel_versions.json";
        let sha = "66cc1bfeb41c6591a5f1d9cde5479b91d9b2aae263ce8a4ba0c29ae58c59b07b";
        let target = temp("net").join("kernel_versions.json");
        download_verified(url, &target, 6333, sha, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::metadata(&target).unwrap().len(), 6333);
        // a partial download with the right etag resumes
        let body = std::fs::read(&target).unwrap();
        std::fs::remove_file(&target).unwrap();
        let part = target.with_file_name("kernel_versions.json.part");
        std::fs::write(&part, &body[..1000]).unwrap();
        // no etag file: treated as changed, restarts cleanly
        download_verified(url, &target, 6333, sha, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), body);
        // wrong size pin is refused before downloading
        std::fs::remove_file(&target).unwrap();
        assert!(download_verified(url, &target, 6334, sha, &mut |_, _| {}).unwrap_err().to_string().contains("expected 6334"));
    }

    /// Extracts a real KDK and compares with a tree made by tools/kdk_*.py, when KDK_DMG and
    /// KDK_REF are set.
    #[test]
    #[ignore]
    fn real_kdk_extract_matches_python() {
        let (Ok(dmg), Ok(reference)) = (std::env::var("KDK_DMG"), std::env::var("KDK_REF")) else { return };
        let out = temp("real").join("out");
        let t = std::time::Instant::now();
        let report = extract(Path::new(&dmg), &out, &mut |_, _| {}).unwrap();
        eprintln!("extracted {} files, {} bytes in {:.1}s, longest path {}", report.files, report.bytes, t.elapsed().as_secs_f64(), report.longest_path);
        assert!(out.join(MARKER).exists());
        let reference = PathBuf::from(reference);
        let mut ours = 0u64;
        let mut stack = vec![out.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().unwrap() != MARKER {
                    let rel = p.strip_prefix(&out).unwrap();
                    let theirs = std::fs::read(reference.join(rel)).unwrap_or_else(|_| panic!("{} missing from the python tree", rel.display()));
                    assert!(std::fs::read(&p).unwrap() == theirs, "{} differs", rel.display());
                    ours += 1;
                }
            }
        }
        let mut theirs = 0u64;
        let mut stack = vec![reference.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if wanted(&p.strip_prefix(&reference).unwrap().to_string_lossy()) {
                    theirs += 1;
                }
            }
        }
        assert_eq!(ours, theirs, "every wanted file in the python tree was extracted");
        assert_eq!(ours, report.files);
    }
}

/// A reader that counts the bytes taken from it.
struct Counting(Cursor<Vec<u8>>, std::sync::Arc<std::sync::atomic::AtomicU64>);
impl Read for Counting {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.0.read(b)?;
        self.1.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(n)
    }
}

#[test]
fn pbzx_bounds_chunks_in_flight() {
    // 50 stored 64 KiB chunks; nobody reads the output for a while
    let chunk = noise(64 << 10, 9);
    let chunks: Vec<(&[u8], bool)> = (0..50).map(|_| (&chunk[..], false)).collect();
    let file = pbzx_file(&chunks);
    let taken = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut p = Pbzx::new(Counting(Cursor::new(file), taken.clone()), 2).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let read = taken.load(std::sync::atomic::Ordering::Relaxed);
    let per_chunk = 16 + chunk.len() as u64;
    assert!(read <= 12 + 4 * per_chunk + per_chunk, "read {} chunks ahead of the consumer", read / per_chunk);
    let mut out = Vec::new();
    p.read_to_end(&mut out).unwrap();
    assert_eq!(out.len(), 50 * chunk.len());
}

#[test]
fn pbzx_dropped_early_does_not_hang() {
    let chunk = noise(64 << 10, 10);
    let chunks: Vec<(&[u8], bool)> = (0..50).map(|_| (&chunk[..], false)).collect();
    let mut p = Pbzx::new(Cursor::new(pbzx_file(&chunks)), 2).unwrap();
    let mut one = [0u8; 10];
    p.read_exact(&mut one).unwrap();
    drop(p);
}
