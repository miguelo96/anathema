use anyhow::{anyhow, bail, Result};

pub fn lzfse_stream(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    lzfse_rust::decode_bytes(data, &mut out).map_err(|e| anyhow!("lzfse: {e:?}"))?;
    Ok(out)
}

pub fn lzvn_raw(data: &[u8], raw: usize) -> Result<Vec<u8>> {
    let mut wrapped = Vec::with_capacity(data.len() + 16);
    wrapped.extend_from_slice(b"bvxn");
    wrapped.extend_from_slice(&(raw as u32).to_le_bytes());
    wrapped.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wrapped.extend_from_slice(data);
    wrapped.extend_from_slice(b"bvx$");
    lzfse_stream(&wrapped, raw)
}

pub fn has_bvx_magic(data: &[u8]) -> bool {
    data.len() >= 4 && matches!(&data[..4], b"bvx2" | b"bvx1" | b"bvxn" | b"bvx-")
}

pub fn lzvn_or_lzfse(data: &[u8], raw: usize) -> Result<Vec<u8>> {
    let out = if has_bvx_magic(data) { lzfse_stream(data, raw)? } else { lzvn_raw(data, raw)? };
    if out.len() != raw {
        bail!("decompressed to {} bytes, expected {raw}", out.len());
    }
    Ok(out)
}

fn u24(b: &[u8], o: usize) -> usize {
    b[o] as usize | (b[o + 1] as usize) << 8 | (b[o + 2] as usize) << 16
}

struct Nibbles<'a> {
    d: &'a [u8],
    pos: usize,
    high: bool,
}

impl Nibbles<'_> {
    fn next(&mut self, end: usize) -> Result<u8> {
        if self.pos >= end {
            bail!("lzbitmap nibble stream overrun");
        }
        let v = if self.high { self.d[self.pos] >> 4 } else { self.d[self.pos] & 15 };
        if self.high {
            self.pos += 1;
        }
        self.high = !self.high;
        Ok(v)
    }
    fn rewind(&mut self) {
        if self.high {
            self.high = false;
        } else {
            self.high = true;
            self.pos -= 1;
        }
    }
}

pub fn lzbitmap(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    if data.len() < 4 || &data[..4] != b"ZBM\x09" {
        bail!("missing lzbitmap magic");
    }
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut p = 4;
    loop {
        if p + 6 > data.len() {
            bail!("truncated lzbitmap chunk header");
        }
        let len = u24(data, p);
        let dlen = u24(data, p + 3);
        if len < 6 || p + len > data.len() || dlen > 0x8000 {
            bail!("bad lzbitmap chunk");
        }
        if dlen == 0 {
            break;
        }
        let chunk = &data[p..p + len];
        if len == dlen + 6 {
            out.extend_from_slice(&chunk[6..]);
        } else {
            lzbitmap_chunk(chunk, dlen, &mut out)?;
        }
        p += len;
    }
    Ok(out)
}

fn lzbitmap_chunk(c: &[u8], dlen: usize, out: &mut Vec<u8>) -> Result<()> {
    let len = c.len();
    if len < 15 + 17 {
        bail!("short lzbitmap chunk");
    }
    let off1 = u24(c, 6);
    let off2 = u24(c, 9);
    let off3 = u24(c, 12);
    if off1 >= len || off2 >= len || off3 >= len {
        bail!("lzbitmap metadata offset out of range");
    }
    let mut data = 15usize;
    let mut m1 = off1;
    let mut m2 = off2;
    let mut m3 = Nibbles { d: c, pos: off3, high: false };
    let mut maps = [(0u8, 0u8); 12];
    let bits_at = len - 17;
    let mut bit = 0usize;
    let rd = |bit: &mut usize| -> Result<u8> {
        let byte = bits_at + *bit / 8;
        if byte >= len {
            bail!("lzbitmap bitmap table overrun");
        }
        let v = (c[byte] >> (*bit % 8)) & 1;
        *bit += 1;
        Ok(v)
    };
    for m in maps.iter_mut() {
        let mut bm = 0u8;
        for i in 0..8 {
            bm |= rd(&mut bit)? << i;
        }
        let mut pb = 0u8;
        for i in 0..2 {
            pb |= rd(&mut bit)? << i;
        }
        if pb > 2 {
            bail!("bad lzbitmap period byte count");
        }
        *m = (bm, pb);
    }
    let mut period = 8usize;
    let mut written = 0usize;
    let apply = |out: &mut Vec<u8>, written: &mut usize, period: &mut usize, data: &mut usize, m1: &mut usize, bm: u8, pb: u8| -> Result<()> {
        if pb > 0 {
            *period = 0;
            for i in 0..pb as usize {
                if *m1 >= len {
                    bail!("lzbitmap period overrun");
                }
                *period |= (c[*m1] as usize) << (i * 8);
                *m1 += 1;
            }
        }
        if *period == 0 {
            bail!("zero lzbitmap period");
        }
        for i in 0..8 {
            if *written == dlen {
                break;
            }
            if bm & (1 << i) != 0 {
                if *data >= len {
                    bail!("lzbitmap literal overrun");
                }
                out.push(c[*data]);
                *data += 1;
            } else {
                if out.len() < *period {
                    bail!("lzbitmap backreference before start");
                }
                let b = out[out.len() - *period];
                out.push(b);
            }
            *written += 1;
        }
        Ok(())
    };
    while written < dlen {
        let num = m3.next(len)?;
        let mut repeat = 1usize;
        if dlen - written > 8 {
            let n = m3.next(len)?;
            if n != 0xf {
                m3.rewind();
            } else {
                let mut total = 4usize;
                let mut n = 0xfu8;
                while n == 0xf {
                    n = m3.next(len)?;
                    total += n as usize;
                }
                repeat = total;
            }
        }
        for _ in 0..repeat {
            if num == 0xf {
                bail!("bad lzbitmap bitmap number");
            }
            if num > 2 {
                let (bm, pb) = maps[(num - 3) as usize];
                apply(out, &mut written, &mut period, &mut data, &mut m1, bm, pb)?;
            } else {
                if m2 >= len {
                    bail!("lzbitmap bitmap stream overrun");
                }
                let bm = c[m2];
                m2 += 1;
                apply(out, &mut written, &mut period, &mut data, &mut m1, bm, num)?;
            }
        }
    }
    Ok(())
}

pub fn block(data: &[u8], raw: usize) -> Result<Vec<u8>> {
    if data.len() >= 4 && &data[..4] == b"ZBM	" {
        let out = lzbitmap(data, raw)?;
        if out.len() != raw {
            bail!("lzbitmap produced {} bytes, expected {raw}", out.len());
        }
        return Ok(out);
    }
    if data.len() == raw {
        return Ok(data.to_vec());
    }
    lzvn_or_lzfse(data, raw)
}
