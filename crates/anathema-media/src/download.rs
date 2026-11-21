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
    let client = http::client()?;
    if target.exists() {
        match sha1 {
            Some(h) if !verify_sha1(&target, h)? => bail!("{} exists but fails sha1", target.display()),
            _ => return Ok(target),
        }
    }
    let head = http::probe(&client, url)?;
    let etag = head.etag.clone().unwrap_or_default();
    let mut have = part.metadata().map(|m| m.len()).unwrap_or(0);
    if have > 0 && std::fs::read_to_string(&meta).unwrap_or_default() != etag {
        eprintln!("object changed, restarting");
        std::fs::remove_file(&part)?;
        have = 0;
    }
    std::fs::write(&meta, &etag)?;
    let mut out = OpenOptions::new().create(true).append(true).open(&part)?;
    let started = Instant::now();
    let base = have;
    let mut report = Instant::now();
    let mut failures = 0u32;
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
                    if report.elapsed() > Duration::from_secs(2) {
                        let rate = (have - base) as f64 / started.elapsed().as_secs_f64();
                        eprint!(
                            "\r{:5.1}%  {:.1} MB/s  {:.2} / {:.2} GB   ",
                            have as f64 * 100.0 / head.len as f64,
                            rate / 1e6,
                            have as f64 / 1e9,
                            head.len as f64 / 1e9
                        );
                        report = Instant::now();
                    }
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
    drop(out);
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
