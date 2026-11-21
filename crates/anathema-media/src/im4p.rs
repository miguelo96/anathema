use anyhow::{bail, Context, Result};

pub struct Tlv<'a> {
    pub tag: u8,
    pub body: &'a [u8],
    pub rest: &'a [u8],
}

pub fn tlv(data: &[u8]) -> Result<Tlv<'_>> {
    if data.len() < 2 {
        bail!("truncated der");
    }
    let tag = data[0];
    let (len, hdr) = if data[1] & 0x80 == 0 {
        (data[1] as usize, 2)
    } else {
        let n = (data[1] & 0x7f) as usize;
        if n == 0 || n > 8 || data.len() < 2 + n {
            bail!("bad der length");
        }
        let mut l = 0usize;
        for &b in &data[2..2 + n] {
            l = (l << 8) | b as usize;
        }
        (l, 2 + n)
    };
    if data.len() < hdr + len {
        bail!("der body exceeds input");
    }
    Ok(Tlv { tag, body: &data[hdr..hdr + len], rest: &data[hdr + len..] })
}

pub fn der_int(body: &[u8]) -> u64 {
    body.iter().fold(0u64, |a, &b| (a << 8) | b as u64)
}

pub struct Im4p {
    pub kind: String,
    pub desc: String,
    pub payload: Vec<u8>,
    pub kbag: Option<Vec<u8>>,
    pub compression: Option<(u64, u64)>,
}

pub fn parse(data: &[u8]) -> Result<Im4p> {
    let top = tlv(data)?;
    if top.tag != 0x30 {
        bail!("not a der sequence");
    }
    let mut cur = top.body;
    let magic = tlv(cur)?;
    if magic.tag != 0x16 || magic.body != b"IM4P" {
        bail!("not an IM4P");
    }
    cur = magic.rest;
    let kind = tlv(cur)?;
    cur = kind.rest;
    let desc = tlv(cur)?;
    cur = desc.rest;
    let payload = tlv(cur)?;
    if payload.tag != 0x04 {
        bail!("im4p payload is not an octet string");
    }
    cur = payload.rest;
    let mut kbag = None;
    let mut compression = None;
    while !cur.is_empty() {
        let t = tlv(cur)?;
        cur = t.rest;
        match t.tag {
            0x04 => kbag = Some(t.body.to_vec()),
            0x30 => {
                let a = tlv(t.body)?;
                let b = tlv(a.rest)?;
                compression = Some((der_int(a.body), der_int(b.body)));
            }
            _ => {}
        }
    }
    Ok(Im4p {
        kind: String::from_utf8_lossy(kind.body).into_owned(),
        desc: String::from_utf8_lossy(desc.body).into_owned(),
        payload: payload.body.to_vec(),
        kbag,
        compression,
    })
}

pub fn decompress(data: &[u8], expected: Option<u64>) -> Result<Vec<u8>> {
    if data.len() >= 4 && matches!(&data[..4], b"bvx2" | b"bvx1" | b"bvxn" | b"bvx-") {
        let mut out = Vec::with_capacity(expected.unwrap_or(0) as usize);
        lzfse_rust::decode_bytes(data, &mut out).map_err(|e| anyhow::anyhow!("lzfse: {e:?}"))?;
        return Ok(out);
    }
    if data.len() >= 8 && &data[..8] == b"complzss" {
        return lzss(data);
    }
    Ok(data.to_vec())
}

fn lzss(data: &[u8]) -> Result<Vec<u8>> {
    let usize_ = u32::from_be_bytes(data[12..16].try_into()?) as usize;
    let csize = u32::from_be_bytes(data[16..20].try_into()?) as usize;
    let src = data.get(0x180..0x180 + csize).context("truncated lzss")?;
    let mut ring = vec![b' '; 4096];
    let mut r = 4096 - 18;
    let mut out = Vec::with_capacity(usize_);
    let mut i = 0;
    let mut flags = 0u32;
    while out.len() < usize_ && i < src.len() {
        flags >>= 1;
        if flags & 0x100 == 0 {
            flags = src[i] as u32 | 0xff00;
            i += 1;
        }
        if flags & 1 != 0 {
            let c = src[i];
            i += 1;
            out.push(c);
            ring[r] = c;
            r = (r + 1) & 4095;
        } else {
            if i + 1 >= src.len() {
                break;
            }
            let a = src[i] as usize;
            let b = src[i + 1] as usize;
            i += 2;
            let pos = a | ((b & 0xf0) << 4);
            let len = (b & 0x0f) + 3;
            for k in 0..len {
                let c = ring[(pos + k) & 4095];
                out.push(c);
                ring[r] = c;
                r = (r + 1) & 4095;
            }
        }
    }
    out.truncate(usize_);
    Ok(out)
}

pub fn decode(data: &[u8]) -> Result<(Im4p, Vec<u8>)> {
    let im = parse(data)?;
    if im.kbag.is_some() {
        bail!("{} payload is encrypted", im.kind);
    }
    let out = decompress(&im.payload, im.compression.map(|c| c.1))?;
    Ok((im, out))
}

pub fn is_im4p(data: &[u8]) -> bool {
    parse(data).is_ok()
}

pub struct Node {
    pub props: Vec<(String, Vec<u8>)>,
    pub children: Vec<Node>,
}

pub fn parse_devicetree(data: &[u8]) -> Result<Node> {
    fn node(d: &[u8], p: &mut usize) -> Result<Node> {
        if *p + 8 > d.len() {
            bail!("truncated devicetree");
        }
        let np = u32::from_le_bytes(d[*p..*p + 4].try_into()?) as usize;
        let nc = u32::from_le_bytes(d[*p + 4..*p + 8].try_into()?) as usize;
        *p += 8;
        let mut props = Vec::with_capacity(np);
        for _ in 0..np {
            if *p + 36 > d.len() {
                bail!("truncated property");
            }
            let name = &d[*p..*p + 32];
            let end = name.iter().position(|&b| b == 0).unwrap_or(32);
            let name = String::from_utf8_lossy(&name[..end]).into_owned();
            let len = (u32::from_le_bytes(d[*p + 32..*p + 36].try_into()?) & 0x7fff_ffff) as usize;
            *p += 36;
            let v = d.get(*p..*p + len).context("property exceeds tree")?.to_vec();
            *p += (len + 3) & !3;
            props.push((name, v));
        }
        let mut children = Vec::with_capacity(nc);
        for _ in 0..nc {
            children.push(node(d, p)?);
        }
        Ok(Node { props, children })
    }
    let mut p = 0;
    node(data, &mut p)
}

impl Node {
    pub fn prop(&self, name: &str) -> Option<&[u8]> {
        self.props.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }
    pub fn name(&self) -> String {
        self.prop("name")
            .map(|v| String::from_utf8_lossy(v.split(|&b| b == 0).next().unwrap_or(&[])).into_owned())
            .unwrap_or_default()
    }
    pub fn dump(&self, depth: usize, out: &mut String) {
        use std::fmt::Write;
        let pad = "  ".repeat(depth);
        let _ = writeln!(out, "{pad}{} ({} props)", self.name(), self.props.len());
        for (n, v) in &self.props {
            let printable = !v.is_empty()
                && v.len() < 96
                && v.iter().take(v.len() - 1).all(|&b| (0x20..0x7f).contains(&b))
                && *v.last().unwrap() == 0;
            if printable {
                let _ = writeln!(out, "{pad}  {n} = \"{}\"", String::from_utf8_lossy(&v[..v.len() - 1]));
            } else if v.len() <= 16 {
                let _ = writeln!(out, "{pad}  {n} = {}", crate::hash::hex(v));
            } else {
                let _ = writeln!(out, "{pad}  {n} = <{} bytes>", v.len());
            }
        }
        for c in &self.children {
            c.dump(depth + 1, out);
        }
    }
}

pub struct MachO {
    pub cpu: u32,
    pub filetype: u32,
    pub ncmds: u32,
    pub entries: Vec<String>,
    pub segments: Vec<(String, u64, u64)>,
}

pub fn parse_macho(d: &[u8]) -> Result<MachO> {
    if d.len() < 32 || u32::from_le_bytes(d[..4].try_into()?) != 0xfeedfacf {
        bail!("not a 64-bit mach-o");
    }
    let rd = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let rd64 = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let ncmds = rd(16);
    let mut p = 32;
    let mut entries = Vec::new();
    let mut segments = Vec::new();
    for _ in 0..ncmds {
        if p + 8 > d.len() {
            break;
        }
        let cmd = rd(p);
        let size = rd(p + 4) as usize;
        if cmd == 0x8000_0035 {
            let off = rd(p + 24) as usize;
            let s = &d[p + off..p + size];
            let e = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            entries.push(String::from_utf8_lossy(&s[..e]).into_owned());
        } else if cmd == 0x19 {
            let n = &d[p + 8..p + 24];
            let e = n.iter().position(|&b| b == 0).unwrap_or(16);
            segments.push((String::from_utf8_lossy(&n[..e]).into_owned(), rd64(p + 24), rd64(p + 32)));
        }
        p += size;
    }
    Ok(MachO { cpu: rd(4), filetype: rd(12), ncmds, entries, segments })
}
