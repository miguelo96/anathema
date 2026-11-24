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
