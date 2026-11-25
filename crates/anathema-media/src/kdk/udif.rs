//! Read-only UDIF (.dmg) access: the `koly` trailer, the `blkx` chunk tables in its XML
//! resource fork, and random access into one partition through decompressed chunks.

use crate::codec::lzfse_stream;
use crate::plist::{self, Value};
use crate::readat::ReadAt;
use anyhow::{anyhow, bail, Context, Result};
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

const SECTOR: u64 = 512;
const CACHE_CHUNKS: usize = 64;

#[derive(Debug, Clone)]
pub struct Partition {
    pub name: String,
    pub sectors: u64,
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    kind: u32,
    /// first sector, relative to the partition
    sector: u64,
    sectors: u64,
    /// absolute offset in the dmg file
    offset: u64,
    length: u64,
}

pub struct Udif {
    r: Arc<dyn ReadAt + Send + Sync>,
    parts: Vec<Partition>,
    chunks: Vec<Arc<Vec<Chunk>>>,
}

fn be32(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(o..o + 4).ok_or_else(|| anyhow!("dmg table truncated"))?.try_into().unwrap()))
}
fn be64(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(b.get(o..o + 8).ok_or_else(|| anyhow!("dmg table truncated"))?.try_into().unwrap()))
}

fn parse_mish(d: &[u8], data_fork: u64) -> Result<(u64, Vec<Chunk>)> {
    if d.get(..4) != Some(b"mish") {
        bail!("dmg partition table has no mish magic");
    }
    let sectors = be64(d, 16)?;
    let data_offset = be64(d, 24)?;
    let n = be32(d, 200)? as usize;
    let mut chunks = Vec::with_capacity(n);
    for i in 0..n {
        let c = 204 + 40 * i;
        let kind = be32(d, c)?;
        if kind == 0xffff_ffff || kind == 0x7fff_fffe {
            continue;
        }
        chunks.push(Chunk {
            kind,
            sector: be64(d, c + 8)?,
            sectors: be64(d, c + 16)?,
            offset: data_fork + data_offset + be64(d, c + 24)?,
            length: be64(d, c + 32)?,
        });
    }
    chunks.sort_by_key(|c| c.sector);
    Ok((sectors, chunks))
}

impl Udif {
    pub fn open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Udif> {
        let len = r.len();
        if len < 512 {
            bail!("file too small to be a dmg");
        }
        let koly = r.read_vec(len - 512, 512)?;
        if &koly[..4] != b"koly" {
            bail!("not a dmg (no koly trailer)");
        }
        let data_fork = be64(&koly, 0x18)?;
        let (xml_off, xml_len) = (be64(&koly, 0xd8)?, be64(&koly, 0xe0)?);
        if xml_len == 0 || xml_off.checked_add(xml_len).is_none_or(|e| e > len) {
            bail!("dmg has no usable partition table");
        }
        let xml = r.read_vec(xml_off, xml_len as usize)?;
        let doc = plist::parse(std::str::from_utf8(&xml).context("dmg partition table is not utf-8")?)?;
        let blkx = doc.path(&["resource-fork", "blkx"]).and_then(Value::as_array).ok_or_else(|| anyhow!("dmg has no blkx table"))?;
        let mut parts = Vec::new();
        let mut chunks = Vec::new();
        for b in blkx {
            let name = b.get("Name").or_else(|| b.get("CFName")).and_then(Value::as_str).unwrap_or("").to_string();
            let data = b.get("Data").and_then(Value::as_data).ok_or_else(|| anyhow!("dmg partition {name} has no data"))?;
            let (sectors, c) = parse_mish(data, data_fork).with_context(|| format!("dmg partition {name}"))?;
            parts.push(Partition { name, sectors });
            chunks.push(Arc::new(c));
        }
        Ok(Udif { r, parts, chunks })
    }

    pub fn partitions(&self) -> &[Partition] {
        &self.parts
    }

    /// Index of the first partition whose name says it holds HFS+ (or HFSX).
    pub fn find_hfs(&self) -> Result<usize> {
        self.parts.iter().position(|p| p.name.contains("Apple_HFS")).ok_or_else(|| anyhow!("dmg has no HFS+ partition"))
    }

    pub fn open_partition(&self, index: usize) -> Result<PartitionReader> {
        let p = self.parts.get(index).ok_or_else(|| anyhow!("dmg has no partition {index}"))?;
        Ok(PartitionReader { r: self.r.clone(), len: p.sectors * SECTOR, chunks: self.chunks[index].clone(), cache: Mutex::new(Vec::new()) })
    }
}

/// Random access to one partition's bytes.
pub struct PartitionReader {
    r: Arc<dyn ReadAt + Send + Sync>,
    len: u64,
    chunks: Arc<Vec<Chunk>>,
    cache: Mutex<Vec<(usize, Arc<Vec<u8>>)>>,
}

fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

impl PartitionReader {
    fn chunk_bytes(&self, i: usize) -> io::Result<Arc<Vec<u8>>> {
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(pos) = cache.iter().position(|(k, _)| *k == i) {
                let hit = cache.remove(pos);
                let data = hit.1.clone();
                cache.push(hit);
                return Ok(data);
            }
        }
        let c = self.chunks[i];
        let raw = (c.sectors * SECTOR) as usize;
        let data = match c.kind {
            0x0 | 0x2 => vec![0u8; raw],
            0x1 => self.r.read_vec(c.offset, raw)?,
            0x8000_0005 => {
                let src = self.r.read_vec(c.offset, c.length as usize)?;
                let mut out = Vec::with_capacity(raw);
                flate2::read::ZlibDecoder::new(&src[..]).read_to_end(&mut out)?;
                out
            }
            0x8000_0007 => {
                let src = self.r.read_vec(c.offset, c.length as usize)?;
                lzfse_stream(&src, raw).map_err(invalid)?
            }
            other => return Err(invalid(format!("dmg chunk type {other:#x} is not supported"))),
        };
        if data.len() != raw {
            return Err(invalid(format!("dmg chunk at sector {} decoded to {} bytes, expected {raw}", c.sector, data.len())));
        }
        let data = Arc::new(data);
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= CACHE_CHUNKS {
                cache.remove(0);
            }
            cache.push((i, data.clone()));
        }
        Ok(data)
    }
}

impl ReadAt for PartitionReader {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let sector = offset / SECTOR;
        let i = self.chunks.partition_point(|c| c.sector + c.sectors <= sector);
        let Some(c) = self.chunks.get(i).filter(|c| c.sector <= sector) else {
            // a gap in the table reads as zeros up to the next chunk
            let next = self.chunks.get(i).map(|c| c.sector * SECTOR).unwrap_or(self.len);
            let n = buf.len().min((next - offset) as usize).max(1).min(buf.len());
            buf[..n].fill(0);
            return Ok(n);
        };
        let start = c.sector * SECTOR;
        let end = (c.sector + c.sectors) * SECTOR;
        let n = buf.len().min((end.min(self.len) - offset) as usize);
        if c.kind == 0x0 || c.kind == 0x2 {
            buf[..n].fill(0);
            return Ok(n);
        }
        let data = self.chunk_bytes(i)?;
        let at = (offset - start) as usize;
        buf[..n].copy_from_slice(&data[at..at + n]);
        Ok(n)
    }

    fn len(&self) -> u64 {
        self.len
    }
}
