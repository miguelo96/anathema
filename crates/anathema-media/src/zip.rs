use crate::readat::{le16, le32, le64, Cursor, ReadAt};
use anyhow::{bail, Context, Result};
use std::io::{self, BufReader, Read, Write};

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub csize: u64,
    pub size: u64,
    pub header: u64,
}

pub struct Zip<'a> {
    pub src: &'a dyn ReadAt,
    pub entries: Vec<Entry>,
}

impl<'a> Zip<'a> {
    pub fn open(src: &'a dyn ReadAt) -> Result<Self> {
        let len = src.len();
        let tail_len = len.min(70_000) as usize;
        let tail = src.read_vec(len - tail_len as u64, tail_len)?;
        let eocd = (0..tail_len.saturating_sub(21))
            .rev()
            .find(|&i| le32(&tail, i) == 0x06054b50)
            .context("no zip end of central directory")?;
        let mut count = le16(&tail, eocd + 10) as u64;
        let mut cd_size = le32(&tail, eocd + 12) as u64;
        let mut cd_off = le32(&tail, eocd + 16) as u64;
        if eocd >= 20 && le32(&tail, eocd - 20) == 0x07064b50 {
            let rec = le64(&tail, eocd - 20 + 8);
            let h = src.read_vec(rec, 56)?;
            if le32(&h, 0) != 0x06064b50 {
                bail!("bad zip64 record");
            }
            count = le64(&h, 32);
            cd_size = le64(&h, 40);
            cd_off = le64(&h, 48);
        }
        if cd_off.checked_add(cd_size).is_none_or(|end| end > len) {
            bail!("zip central directory lies outside the file");
        }
        let cd = src.read_vec(cd_off, cd_size as usize)?;
        // `count` comes from the file; each entry takes at least 46 bytes of the directory.
        let mut entries = Vec::with_capacity((count as usize).min(cd.len() / 46));
        let mut p = 0usize;
        while p + 46 <= cd.len() && le32(&cd, p) == 0x02014b50 {
            let method = le16(&cd, p + 10);
            let mut csize = le32(&cd, p + 20) as u64;
            let mut size = le32(&cd, p + 24) as u64;
            let nlen = le16(&cd, p + 28) as usize;
            let elen = le16(&cd, p + 30) as usize;
            let clen = le16(&cd, p + 32) as usize;
            let mut header = le32(&cd, p + 42) as u64;
            let name = cd.get(p + 46..p + 46 + nlen).context("truncated zip central directory")?;
            let name = String::from_utf8_lossy(name).into_owned();
            let extra = cd.get(p + 46 + nlen..p + 46 + nlen + elen).context("truncated zip central directory")?;
            let mut e = 0;
            while e + 4 <= extra.len() {
                let id = le16(extra, e);
                let sz = le16(extra, e + 2) as usize;
                if id == 1 {
                    let field = extra.get(e + 4..e + 4 + sz).context("truncated zip64 extra field")?;
                    let mut q = 0;
                    let mut next = || -> Result<u64> {
                        let v = field.get(q..q + 8).context("short zip64 extra field")?;
                        q += 8;
                        Ok(le64(v, 0))
                    };
                    if size == 0xffff_ffff {
                        size = next()?;
                    }
                    if csize == 0xffff_ffff {
                        csize = next()?;
                    }
                    if header == 0xffff_ffff {
                        header = next()?;
                    }
                }
                e += 4 + sz;
            }
            entries.push(Entry { name, method, csize, size, header });
            p += 46 + nlen + elen + clen;
        }
        Ok(Self { src, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn data_offset(&self, e: &Entry) -> Result<u64> {
        let h = self.src.read_vec(e.header, 30)?;
        if le32(&h, 0) != 0x04034b50 {
            bail!("bad local header for {}", e.name);
        }
        Ok(e.header + 30 + le16(&h, 26) as u64 + le16(&h, 28) as u64)
    }

    pub fn reader(&self, e: &Entry) -> Result<Box<dyn Read + '_>> {
        let start = self.data_offset(e)?;
        let raw = BufReader::with_capacity(4 << 20, Cursor::new(self.src, start, start + e.csize));
        match e.method {
            0 => Ok(Box::new(raw)),
            8 => Ok(Box::new(flate2::read::DeflateDecoder::new(raw))),
            m => bail!("unsupported zip method {m} for {}", e.name),
        }
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self.find(name).with_context(|| format!("{name} is not in the archive"))?.clone();
        let mut v = Vec::with_capacity(e.size as usize);
        self.reader(&e)?.read_to_end(&mut v)?;
        Ok(v)
    }

    pub fn copy_to(&self, e: &Entry, out: &mut dyn Write, progress: &mut dyn FnMut(u64)) -> Result<u64> {
        let mut r = self.reader(e)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = match r.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            total += n as u64;
            progress(total);
        }
        Ok(total)
    }
}
