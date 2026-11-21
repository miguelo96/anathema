use crate::readat::{le32, le64, FileAt, ReadAt};
use anyhow::{bail, ensure, Context, Result};
use std::path::Path;

struct Mapping {
    addr: u64,
    size: u64,
    offset: u64,
    file: usize,
}

pub struct Dsc {
    files: Vec<FileAt>,
    maps: Vec<Mapping>,
    pub images: Vec<(u64, String)>,
}

pub struct Section {
    pub segment: String,
    pub name: String,
    pub addr: u64,
    pub size: u64,
}

impl Dsc {
    pub fn open(dir: &Path, prefix: &str) -> Result<Self> {
        let mut names: Vec<String> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
        ensure!(names.first().map(String::as_str) == Some(prefix), "{prefix} not found in {}", dir.display());
        let mut files = Vec::new();
        let mut maps = Vec::new();
        for name in &names {
            let f = FileAt::open(&dir.join(name))?;
            let head = f.read_vec(0, 0x200.min(f.len() as usize))?;
            if head.len() < 0x20 || !head.starts_with(b"dyld_v1") {
                continue;
            }
            let moff = le32(&head, 16) as u64;
            let mcount = le32(&head, 20) as u64;
            let idx = files.len();
            for i in 0..mcount {
                let m = f.read_vec(moff + i * 32, 32)?;
                maps.push(Mapping { addr: le64(&m, 0), size: le64(&m, 8), offset: le64(&m, 16), file: idx });
            }
            files.push(f);
        }
        let main = &files[0];
        let head = main.read_vec(0, 0x1c8)?;
        let (ioff, icount) = (le32(&head, 0x1c0) as u64, le32(&head, 0x1c4) as u64);
        let mut images = Vec::new();
        let table = main.read_vec(ioff, (icount * 32) as usize)?;
        for i in 0..icount as usize {
            let addr = le64(&table, i * 32);
            let poff = le32(&table, i * 32 + 24) as u64;
            let raw = main.read_vec(poff, 512.min((main.len() - poff) as usize))?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            images.push((addr, String::from_utf8_lossy(&raw[..end]).into_owned()));
        }
        Ok(Self { files, maps, images })
    }

    pub fn read_vm(&self, addr: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; len];
        let mut done = 0usize;
        while done < len {
            let a = addr + done as u64;
            let m = self
                .maps
                .iter()
                .find(|m| a >= m.addr && a < m.addr + m.size)
                .with_context(|| format!("address {a:#x} is not mapped by the cache"))?;
            let n = (len - done).min((m.addr + m.size - a) as usize);
            self.files[m.file].read_exact_at(m.offset + (a - m.addr), &mut out[done..done + n])?;
            done += n;
        }
        Ok(out)
    }

    pub fn find(&self, needle: &str) -> Vec<&(u64, String)> {
        self.images.iter().filter(|(_, p)| p.contains(needle)).collect()
    }

    pub fn sections(&self, image: u64) -> Result<Vec<Section>> {
        let head = self.read_vm(image, 0x4000)?;
        ensure!(le32(&head, 0) == 0xfeedfacf, "image is not a 64-bit mach-o");
        let ncmds = le32(&head, 16);
        let mut p = 32usize;
        let mut out = Vec::new();
        for _ in 0..ncmds {
            if p + 8 > head.len() {
                bail!("load commands exceed the probed header");
            }
            let cmd = le32(&head, p);
            let size = le32(&head, p + 4) as usize;
            if cmd == 0x19 {
                let nsects = le32(&head, p + 64) as usize;
                for s in 0..nsects {
                    let o = p + 72 + s * 80;
                    let name = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(16)]).into_owned();
                    out.push(Section {
                        name: name(&head[o..o + 16]),
                        segment: name(&head[o + 16..o + 32]),
                        addr: le64(&head, o + 32),
                        size: le64(&head, o + 40),
                    });
                }
            }
            p += size;
        }
        Ok(out)
    }
}
