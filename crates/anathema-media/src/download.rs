use crate::hash;
use crate::http;
use anyhow::{bail, Context, Result};
use reqwest::{header, StatusCode};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn default_out() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("D:\\anathema-media")
    } else {
        PathBuf::from("/mnt/d/anathema-media")
    }
}

pub fn on_root_fs(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else { return false };
        let probe = {
            let mut p = path.to_path_buf();
            while !p.exists() {
                if !p.pop() {
                    break;
                }
            }
            p.canonicalize().unwrap_or(p)
        };
        let mut best: Option<(usize, bool)> = None;
        for line in mounts.lines() {
            let mut f = line.split_whitespace();
            let (Some(_dev), Some(mp), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
            if probe.starts_with(mp) {
                let native = matches!(fs, "ext4" | "overlay" | "btrfs" | "xfs");
                if best.is_none_or(|(l, _)| mp.len() >= l) {
                    best = Some((mp.len(), native));
                }
            }
        }
        best.is_some_and(|(_, native)| native)
    }
    #[cfg(not(unix))]
    {
        let s = path.to_string_lossy().to_ascii_lowercase();
        s.starts_with("c:")
    }
}

pub fn guard_out(out: &Path, explicit: bool) -> Result<()> {
    if !explicit && on_root_fs(out) {
        bail!(
            "{} is not on the data drive (is D: mounted?); pass --out to override",
            out.display()
        );
    }
    Ok(())
}

pub fn verify_sha1(path: &Path, expected: &str) -> Result<bool> {
    let f = File::open(path)?;
    let total = f.metadata()?.len();
    let mut last = Instant::now();
    let got = hash::sha1_reader(f, &mut |n| {
        if last.elapsed() > Duration::from_secs(2) {
            eprint!("\rverifying {:5.1}%", n as f64 * 100.0 / total as f64);
            last = Instant::now();
        }
    })?;
    eprintln!("\rverified sha1 {got}");
    Ok(got.eq_ignore_ascii_case(expected))
}

pub fn download(url: &str, dir: &Path, sha1: Option<&str>) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = url.rsplit('/').next().context("bad url")?;
    let target = dir.join(name);
    let part = dir.join(format!("{name}.part"));
    let meta = dir.join(format!("{name}.part.etag"));
    if target.exists() {
        match sha1 {
            Some(h) if !verify_sha1(&target, h)? => bail!("{} exists but fails sha1", target.display()),
            _ => return Ok(target),
        }
    }
    let started = Instant::now();
    let mut base = None;
    let mut report = Instant::now();
    resume_fetch(url, &part, &meta, None, &mut |have, len| {
        let from = *base.get_or_insert(have);
        if report.elapsed() > Duration::from_secs(2) {
            let rate = (have - from) as f64 / started.elapsed().as_secs_f64();
            eprint!(
                "\r{:5.1}%  {:.1} MB/s  {:.2} / {:.2} GB   ",
                have as f64 * 100.0 / len as f64,
                rate / 1e6,
                have as f64 / 1e9,
                len as f64 / 1e9
            );
            report = Instant::now();
        }
    })?;
    eprintln!();
    if let Some(h) = sha1 {
        if !verify_sha1(&part, h)? {
            bail!("sha1 mismatch for {}", part.display());
        }
    }
    std::fs::rename(&part, &target)?;
    let _ = std::fs::remove_file(&meta);
    Ok(target)
}

/// Downloads `url` into `part`, resuming what is already there when the server's ETag still
/// matches the one saved in `meta`. With `expect_len`, a server that reports any other size is
/// an error before anything is fetched. `progress` gets (bytes on disk, total).
fn resume_fetch(url: &str, part: &Path, meta: &Path, expect_len: Option<u64>, progress: &mut dyn FnMut(u64, u64)) -> Result<u64> {
    let client = http::client()?;
    let head = http::probe(&client, url)?;
    if let Some(want) = expect_len {
        if head.len != want {
            bail!("{url} is {} bytes, expected {want}", head.len);
        }
    }
    let etag = head.etag.clone().unwrap_or_default();
    let mut have = part.metadata().map(|m| m.len()).unwrap_or(0);
    if have > 0 && (std::fs::read_to_string(meta).unwrap_or_default() != etag || have > head.len) {
        eprintln!("object changed, restarting");
        std::fs::remove_file(part)?;
        have = 0;
    }
    std::fs::write(meta, &etag)?;
    let mut out = OpenOptions::new().create(true).append(true).open(part)?;
    let mut failures = 0u32;
    progress(have, head.len);
    while have < head.len {
        let mut req = client.get(url).header(header::ACCEPT_ENCODING, "identity").header(header::RANGE, format!("bytes={have}-"));
        if !etag.is_empty() {
            req = req.header(header::IF_RANGE, &etag);
        }
        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                failures += 1;
                if failures > 20 {
                    return Err(e).context("download failed");
                }
                eprintln!("\nretrying: {e}");
                std::thread::sleep(Duration::from_secs((1u64 << failures.min(5)).min(30)));
                continue;
            }
        };
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            bail!("server answered {} to a ranged request", resp.status());
        }
        let mut resp = resp;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            match resp.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    out.write_all(&buf[..n])?;
                    have += n as u64;
                    failures = 0;
                    progress(have, head.len);
                }
                Err(e) => {
                    failures += 1;
                    if failures > 20 {
                        return Err(e).context("download failed");
                    }
                    eprintln!("\nconnection dropped: {e}");
                    std::thread::sleep(Duration::from_secs((1u64 << failures.min(5)).min(30)));
                    break;
                }
            }
        }
        out.flush()?;
    }
    out.sync_all()?;
    Ok(head.len)
}

/// True when the file's sha256 is `expected`; otherwise the file is deleted and false returned.
pub fn verify_or_discard(path: &Path, expected: &str) -> Result<bool> {
    let got = hash::sha256_reader(File::open(path).with_context(|| format!("opening {}", path.display()))?, &mut |_| {})?;
    if got.eq_ignore_ascii_case(expected) {
        return Ok(true);
    }
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(false)
}

/// Downloads `url` to `target` unless a file with the right sha256 is already there. The data
/// goes to `<target>.part` first (resumable) and is renamed only once its size and sha256 match.
pub fn download_verified(url: &str, target: &Path, size: u64, sha256: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    if target.exists() && verify_or_discard(target, sha256)? {
        progress(size, size);
        return Ok(());
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = target.file_name().context("download target has no file name")?.to_string_lossy().into_owned();
    let part = target.with_file_name(format!("{name}.part"));
    let meta = target.with_file_name(format!("{name}.part.etag"));
    resume_fetch(url, &part, &meta, Some(size), progress)?;
    if !verify_or_discard(&part, sha256)? {
        let _ = std::fs::remove_file(&meta);
        bail!("{url} downloaded but its sha256 is not {sha256}; the download was deleted, try again");
    }
    std::fs::rename(&part, target).with_context(|| format!("renaming {}", part.display()))?;
    let _ = std::fs::remove_file(&meta);
    Ok(())
}
