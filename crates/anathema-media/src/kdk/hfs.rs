//! Read-only HFS+/HFSX: enough to find a file by path and read its data fork.

use crate::readat::ReadAt;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

const ROOT_FOLDER: u32 = 2;
const EXTENTS_FILE: u32 = 3;

#[derive(Debug, Clone, Copy, Default)]
struct Fork {
    logical: u64,
    total_blocks: u32,
    extents: [(u32, u32); 8],
}

#[derive(Debug, Clone, Copy)]
enum Record {
    Folder(u32),
    File { id: u32, data: Fork, rsrc_logical: u64 },
}

pub struct Hfs {
    r: Arc<dyn ReadAt + Send + Sync>,
    block: u64,
    extents_file: Fork,
    /// (parent folder id, name) to record
    catalog: HashMap<(u32, String), Record>,
}

fn be16(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(b.get(o..o + 2).ok_or_else(|| anyhow!("hfs record truncated"))?.try_into().unwrap()))
}
fn be32(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(o..o + 4).ok_or_else(|| anyhow!("hfs record truncated"))?.try_into().unwrap()))
}
fn be64(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(b.get(o..o + 8).ok_or_else(|| anyhow!("hfs record truncated"))?.try_into().unwrap()))
}

fn fork(b: &[u8], o: usize) -> Result<Fork> {
    let mut f = Fork { logical: be64(b, o)?, total_blocks: be32(b, o + 12)?, ..Default::default() };
    for (i, e) in f.extents.iter_mut().enumerate() {
        *e = (be32(b, o + 16 + 8 * i)?, be32(b, o + 20 + 8 * i)?);
    }
    Ok(f)
}

/// Byte ranges of a fork on the volume: (logical start, volume offset, length).
fn runs(block: u64, extents: &[(u32, u32)]) -> Vec<(u64, u64, u64)> {
    let mut out = Vec::new();
    let mut at = 0u64;
    for &(start, count) in extents.iter().filter(|e| e.1 > 0) {
        let len = count as u64 * block;
        out.push((at, start as u64 * block, len));
        at += len;
    }
    out
}

/// A fork read through its extents.
pub struct HfsFile {
    r: Arc<dyn ReadAt + Send + Sync>,
    len: u64,
    runs: Vec<(u64, u64, u64)>,
}

impl ReadAt for HfsFile {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let i = self.runs.partition_point(|&(at, _, len)| at + len <= offset);
        let Some(&(at, vol, len)) = self.runs.get(i) else {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "hfs file extents end early"));
        };
        let n = buf.len().min((at + len - offset) as usize).min((self.len - offset) as usize);
        self.r.read_at(vol + offset - at, &mut buf[..n])
    }
    fn len(&self) -> u64 {
        self.len
    }
}

/// Calls `f` on every leaf record of a B-tree file.
fn each_leaf(file: &HfsFile, mut f: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    let head = file.read_vec(0, 512).context("b-tree header node")?;
    let first_leaf = be32(&head, 14 + 10)?;
    let node_size = be16(&head, 14 + 18)? as u64;
    if node_size < 512 {
        bail!("b-tree node size {node_size}");
    }
    let mut node = first_leaf;
    let mut guard = 0u32;
    while node != 0 {
        guard += 1;
        if guard > 10_000_000 {
            bail!("b-tree leaf chain loops");
        }
        let n = file.read_vec(node as u64 * node_size, node_size as usize)?;
        if n[8] as i8 != -1 {
            bail!("b-tree node {node} in the leaf chain is not a leaf");
        }
        let count = be16(&n, 10)? as usize;
        for i in 0..count {
            let start = be16(&n, node_size as usize - 2 * (i + 1))? as usize;
            let end = be16(&n, node_size as usize - 2 * (i + 2))? as usize;
            f(n.get(start..end.max(start)).ok_or_else(|| anyhow!("b-tree record outside its node"))?)?;
        }
        node = be32(&n, 0)?;
    }
    Ok(())
}

impl Hfs {
    pub fn open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Hfs> {
        let vh = r.read_vec(1024, 512).context("hfs volume header")?;
        if &vh[..2] != b"H+" && &vh[..2] != b"HX" {
            bail!("not an HFS+ volume");
        }
        let block = be32(&vh, 40)? as u64;
        let extents_file = fork(&vh, 192)?;
        let catalog_fork = fork(&vh, 272)?;
        let mut hfs = Hfs { r, block, extents_file, catalog: HashMap::new() };
        let catalog = hfs.fork_file(4, catalog_fork)?;
        let mut map = HashMap::new();
        each_leaf(&catalog, |rec| {
            let key_len = be16(rec, 0)? as usize;
            let parent = be32(rec, 2)?;
            let name_len = be16(rec, 6)? as usize;
            let units: Vec<u16> = (0..name_len).map(|i| be16(rec, 8 + 2 * i)).collect::<Result<_>>()?;
            let name = String::from_utf16_lossy(&units);
            let d = rec.get(2 + key_len..).ok_or_else(|| anyhow!("catalog record truncated"))?;
            match be16(d, 0)? {
                1 => {
                    map.insert((parent, name), Record::Folder(be32(d, 8)?));
                }
                2 => {
                    let data = fork(d, 88)?;
                    let rsrc_logical = be64(d, 168)?;
                    map.insert((parent, name), Record::File { id: be32(d, 8)?, data, rsrc_logical });
                }
                _ => {}
            }
            Ok(())
        })?;
        hfs.catalog = map;
        Ok(hfs)
    }

    /// All extents of a fork, with the extents overflow file when the inline eight fall short.
    fn fork_file(&self, id: u32, f: Fork) -> Result<HfsFile> {
        let mut extents: Vec<(u32, u32)> = f.extents.to_vec();
        let inline: u64 = extents.iter().map(|e| e.1 as u64).sum();
        if inline < f.total_blocks as u64 {
            if id == EXTENTS_FILE || id == 4 {
                bail!("hfs system file {id} needs the extents overflow file");
            }
            let ov = self.fork_file(EXTENTS_FILE, self.extents_file)?;
            let mut more: Vec<(u32, [(u32, u32); 8])> = Vec::new();
            each_leaf(&ov, |rec| {
                // key: keyLength, forkType, pad, fileID, startBlock
                if rec.len() >= 12 + 64 && rec[2] == 0 && be32(rec, 4)? == id {
                    let start = be32(rec, 8)?;
                    let mut e = [(0u32, 0u32); 8];
                    for (i, x) in e.iter_mut().enumerate() {
                        *x = (be32(rec, 12 + 8 * i)?, be32(rec, 16 + 8 * i)?);
                    }
                    more.push((start, e));
                }
                Ok(())
            })?;
            more.sort_by_key(|m| m.0);
            for (_, e) in more {
                extents.extend(e);
            }
            let have: u64 = extents.iter().map(|e| e.1 as u64).sum();
            if have < f.total_blocks as u64 {
                bail!("hfs file {id}: extents cover {have} of {} blocks", f.total_blocks);
            }
        }
        Ok(HfsFile { r: self.r.clone(), len: f.logical, runs: runs(self.block, &extents) })
    }

    /// Opens the data fork of the file at `path` ('/' separated, from the volume root).
    pub fn open_file(&self, path: &str) -> Result<HfsFile> {
        let mut parent = ROOT_FOLDER;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let rec = self.catalog.get(&(parent, part.to_string())).ok_or_else(|| anyhow!("{path}: {part} not found on the volume"))?;
            match (*rec, i + 1 == parts.len()) {
                (Record::Folder(id), false) => parent = id,
                (Record::File { id, data, rsrc_logical }, true) => {
                    if data.logical == 0 && rsrc_logical > 0 {
                        bail!("{path}: file is compressed (decmpfs), not supported");
                    }
                    return self.fork_file(id, data);
                }
                (Record::Folder(_), true) => bail!("{path} is a folder"),
                (Record::File { .. }, false) => bail!("{path}: {part} is a file"),
            }
        }
        bail!("{path}: empty path")
    }
}
