//! Streaming reader for odc ("070707") cpio archives, the format inside Apple package payloads.

use anyhow::{bail, Context, Result};
use std::io::{self, Read};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path with a leading `./` removed.
    pub name: String,
    pub mode: u32,
    pub size: u64,
}

fn octal(field: &[u8], what: &str) -> Result<u64> {
    let s = std::str::from_utf8(field).ok().filter(|s| s.bytes().all(|b| (b'0'..=b'7').contains(&b)));
    let s = s.with_context(|| format!("cpio header {what} is not octal"))?;
    Ok(u64::from_str_radix(s, 8)?)
}

/// Calls `f` for every entry with a reader over its data (which `f` may read in part or not at
/// all) until the `TRAILER!!!` entry. Returns the number of entries seen.
pub fn for_each(mut r: impl Read, mut f: impl FnMut(&Entry, &mut dyn Read) -> Result<()>) -> Result<u64> {
    let mut count = 0u64;
    loop {
        let mut h = [0u8; 76];
        r.read_exact(&mut h).context("cpio archive ends without a trailer")?;
        if &h[..6] != b"070707" {
            bail!("bad cpio header magic after {count} entries");
        }
        let mode = octal(&h[18..24], "mode")? as u32;
        let namesize = octal(&h[59..65], "name size")? as usize;
        let size = octal(&h[65..76], "file size")?;
        let mut name = vec![0u8; namesize];
        r.read_exact(&mut name).context("cpio name truncated")?;
        if name.last() == Some(&0) {
            name.pop();
        }
        let name = String::from_utf8(name).context("cpio name is not utf-8")?;
        if name == "TRAILER!!!" {
            return Ok(count);
        }
        let name = name.strip_prefix("./").unwrap_or(&name).to_string();
        let entry = Entry { name, mode, size };
        let mut body = (&mut r).take(size);
        f(&entry, &mut body)?;
        io::copy(&mut body, &mut io::sink())?;
        if body.limit() != 0 {
            bail!("cpio data for {} truncated ({} of {size} bytes missing)", entry.name, body.limit());
        }
        count += 1;
    }
}
