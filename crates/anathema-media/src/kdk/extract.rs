//! Pulls the kernel and the kexts out of a KDK dmg: UDIF > HFS+ > KernelDebugKit.pkg (xar) >
//! KDK.pkg/Payload (pbzx) > cpio, writing only what the kext linker needs.

use super::{cpio, hfs::Hfs, pbzx::Pbzx, pin::KERNEL_PATH, udif::Udif, xar::Xar};
use crate::readat::{FileAt, ReadAt};
use anyhow::{bail, Context, Result};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Written last, so a folder holding it was extracted completely.
pub const MARKER: &str = ".anathema-extracted";
const PKG: &str = "KernelDebugKit.pkg";
const PAYLOAD: &str = "KDK.pkg/Payload";
const EXTENSIONS: &str = "System/Library/Extensions/";

#[derive(Debug, Clone, Default)]
pub struct ExtractReport {
    pub files: u64,
    pub bytes: u64,
    /// Longest relative path written, in characters (Windows path length budget).
    pub longest_path: usize,
}

/// The kernel, and regular files inside a `.kext` (but not a `.dSYM`) under the Extensions
/// folder. Paths with empty, `.` or `..` components or backslashes are never accepted.
pub fn wanted(name: &str) -> bool {
    if name.contains('\\') || name.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return false;
    }
    if name == KERNEL_PATH {
        return true;
    }
    let Some(rest) = name.strip_prefix(EXTENSIONS) else { return false };
    let parts: Vec<&str> = rest.split('/').collect();
    let dirs = &parts[..parts.len() - 1];
    dirs.iter().any(|c| c.ends_with(".kext")) && !dirs.iter().any(|c| c.ends_with(".dSYM"))
}

fn is_empty_dir(p: &Path) -> Result<bool> {
    Ok(std::fs::read_dir(p)?.next().is_none())
}

/// Extracts into `out`, which must not exist yet or be an empty folder. `progress` gets
/// (compressed payload bytes read, payload size).
pub fn extract(dmg: &Path, out: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<ExtractReport> {
    if out.exists() && !is_empty_dir(out).with_context(|| format!("reading {}", out.display()))? {
        bail!("{} is not empty; extract needs a new folder", out.display());
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let file: Arc<dyn ReadAt + Send + Sync> = Arc::new(FileAt::open(dmg).with_context(|| format!("opening {}", dmg.display()))?);
    let udif = Udif::open(file).context("reading the dmg")?;
    let part: Arc<dyn ReadAt + Send + Sync> = Arc::new(udif.open_partition(udif.find_hfs()?)?);
    let hfs = Hfs::open(part).context("reading the dmg's HFS+ volume")?;
    let pkg: Arc<dyn ReadAt + Send + Sync> = Arc::new(hfs.open_file(PKG)?);
    let xar = Xar::open(pkg).with_context(|| format!("reading {PKG}"))?;
    let payload = xar.file(PAYLOAD)?;
    let total = payload.length;
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let stream = Pbzx::new(xar.reader(&payload)?, threads)?;
    let consumed = stream.consumed();
    let mut report = ExtractReport::default();
    cpio::for_each(stream, |e, body| {
        progress(consumed.load(Ordering::Relaxed), total);
        if e.mode & 0o170000 != 0o100000 || !wanted(&e.name) {
            return Ok(());
        }
        let dest = out.join(&e.name);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = dest.with_extension("anathema-tmp");
        let mut w = BufWriter::new(std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?);
        let n = std::io::copy(body, &mut w)?;
        w.flush()?;
        drop(w);
        if n != e.size {
            bail!("{}: wrote {n} of {} bytes", e.name, e.size);
        }
        std::fs::rename(&tmp, &dest).with_context(|| format!("renaming {}", tmp.display()))?;
        report.files += 1;
        report.bytes += n;
        report.longest_path = report.longest_path.max(e.name.chars().count());
        Ok(())
    })?;
    progress(total, total);
    if !out.join(KERNEL_PATH).exists() && report.files == 0 {
        bail!("the KDK payload had no kernel or kexts");
    }
    std::fs::write(out.join(MARKER), format!("files {}\nbytes {}\n", report.files, report.bytes))?;
    Ok(report)
}
