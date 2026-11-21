use std::io::{self, Read};

pub trait ReadAt {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_exact_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let n = self.read_at(offset, buf)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
            }
            offset += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }
    fn read_vec(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_exact_at(offset, &mut v)?;
        Ok(v)
    }
}

pub struct FileAt {
    file: std::fs::File,
    len: u64,
}

impl FileAt {
    pub fn open(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl ReadAt for FileAt {
    #[cfg(unix)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(&self.file, buf, offset)
    }
    #[cfg(windows)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        std::os::windows::fs::FileExt::seek_read(&self.file, buf, offset)
    }
    fn len(&self) -> u64 {
        self.len
    }
}

impl ReadAt for Vec<u8> {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let o = offset as usize;
        if o >= self.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.len() - o);
        buf[..n].copy_from_slice(&self[o..o + n]);
        Ok(n)
    }
    fn len(&self) -> u64 {
        Vec::len(self) as u64
    }
}

pub struct Section<'a> {
    pub inner: &'a dyn ReadAt,
    pub start: u64,
    pub size: u64,
}

impl ReadAt for Section<'_> {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.size {
            return Ok(0);
        }
        let n = buf.len().min((self.size - offset) as usize);
        self.inner.read_at(self.start + offset, &mut buf[..n])
    }
    fn len(&self) -> u64 {
        self.size
    }
}

pub struct Cursor<'a> {
    pub inner: &'a dyn ReadAt,
    pub pos: u64,
    pub end: u64,
}

impl<'a> Cursor<'a> {
    pub fn new(inner: &'a dyn ReadAt, start: u64, end: u64) -> Self {
        Self { inner, pos: start, end }
    }
}

impl Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.end {
            return Ok(0);
        }
        let n = buf.len().min((self.end - self.pos) as usize);
        let got = self.inner.read_at(self.pos, &mut buf[..n])?;
        self.pos += got as u64;
        Ok(got)
    }
}

pub fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
pub fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
pub fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
pub fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
pub fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}
