//! Read-only xar (.pkg) access: the zlib table of contents and file data in the heap.

use crate::readat::ReadAt;
use anyhow::{anyhow, bail, Context, Result};
use std::io::{self, Read};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XarFile {
    /// Absolute offset of the file's data in the archive.
    pub offset: u64,
    pub length: u64,
    pub encoding: String,
}

pub struct Xar {
    r: Arc<dyn ReadAt + Send + Sync>,
    files: Vec<(String, XarFile)>,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[at..].find('"')? + at;
    Some(unescape(&tag[at..end]))
}

#[derive(Default)]
struct Frame {
    name: String,
    kind: String,
    offset: Option<u64>,
    length: Option<u64>,
    encoding: String,
}

/// Walks the TOC XML: `<file>` elements nest, and each has `<name>`, `<type>` and a `<data>`
/// with `<offset>`, `<length>` and `<encoding style=…/>`.
fn parse_toc(xml: &str, heap: u64) -> Result<Vec<(String, XarFile)>> {
    let mut out = Vec::new();
    let mut files: Vec<Frame> = Vec::new();
    let mut elems: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(p) = xml[i..].find('<') {
        let text = &xml[i..i + p];
        i += p;
        let end = i + xml[i..].find('>').ok_or_else(|| anyhow!("xar toc: unterminated tag"))?;
        let tag = &xml[i + 1..end];
        i = end + 1;
        if !text.trim().is_empty() {
            let parent = elems.len().checked_sub(2).map(|k| elems[k].as_str());
            if let (Some(f), Some(el)) = (files.last_mut(), elems.last()) {
                let t = text.trim();
                match (el.as_str(), parent) {
                    ("name", Some("file")) => f.name = unescape(t),
                    ("type", Some("file")) => f.kind = t.to_string(),
                    ("offset", Some("data")) if elems.len() >= 3 && elems[elems.len() - 3] == "file" => f.offset = Some(t.parse()?),
                    ("length", Some("data")) if elems.len() >= 3 && elems[elems.len() - 3] == "file" => f.length = Some(t.parse()?),
                    _ => {}
                }
            }
        }
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if elems.pop().as_deref() != Some(name) {
                bail!("xar toc: unbalanced </{name}>");
            }
            if name == "file" {
                let f = files.pop().unwrap();
                let path: Vec<&str> = files.iter().map(|f| f.name.as_str()).chain([f.name.as_str()]).collect();
                if f.kind == "file" {
                    if let (Some(o), Some(l)) = (f.offset, f.length) {
                        out.push((path.join("/"), XarFile { offset: heap + o, length: l, encoding: f.encoding }));
                    }
                }
            }
            continue;
        }
        let selfclose = tag.ends_with('/');
        let name = tag.trim_end_matches('/').split_whitespace().next().unwrap_or("").to_string();
        if name == "encoding" && elems.last().map(String::as_str) == Some("data") && elems.len() >= 2 && elems[elems.len() - 2] == "file" {
            if let Some(f) = files.last_mut() {
                f.encoding = attr(tag, "style").unwrap_or_default();
            }
        }
        if !selfclose {
            if name == "file" {
                files.push(Frame::default());
            }
            elems.push(name);
        }
    }
    Ok(out)
}

impl Xar {
    pub fn open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Xar> {
        let h = r.read_vec(0, 28).context("xar header")?;
        if &h[..4] != b"xar!" {
            bail!("not a xar archive");
        }
        let header_size = u16::from_be_bytes([h[4], h[5]]) as u64;
        let toc_c = u64::from_be_bytes(h[8..16].try_into().unwrap());
        let toc_u = u64::from_be_bytes(h[16..24].try_into().unwrap());
        if toc_c > 64 << 20 || toc_u > 256 << 20 {
            bail!("xar table of contents is implausibly large");
        }
        let raw = r.read_vec(header_size, toc_c as usize).context("xar table of contents")?;
        let mut toc = String::with_capacity(toc_u as usize);
        flate2::read::ZlibDecoder::new(&raw[..]).read_to_string(&mut toc).context("xar table of contents")?;
        let files = parse_toc(&toc, header_size + toc_c)?;
        Ok(Xar { r, files })
    }

    pub fn file(&self, path: &str) -> Result<XarFile> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, f)| f.clone()).ok_or_else(|| anyhow!("{path} is not in the package"))
    }

    /// The paths of every regular file in the archive.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(p, _)| p.as_str())
    }

    pub fn reader(&self, f: &XarFile) -> Result<XarReader> {
        if f.encoding != "application/octet-stream" {
            bail!("package data encoded as {} is not supported", f.encoding);
        }
        if f.offset.checked_add(f.length).is_none_or(|e| e > self.r.len()) {
            bail!("package data runs past the end of the archive");
        }
        Ok(XarReader { r: self.r.clone(), pos: f.offset, end: f.offset + f.length })
    }
}

/// Sequential reader over one file's data in the archive.
pub struct XarReader {
    r: Arc<dyn ReadAt + Send + Sync>,
    pos: u64,
    end: u64,
}

impl Read for XarReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min((self.end - self.pos) as usize);
        if n == 0 {
            return Ok(0);
        }
        let got = self.r.read_at(self.pos, &mut buf[..n])?;
        if got == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "package data truncated"));
        }
        self.pos += got as u64;
        Ok(got)
    }
}
