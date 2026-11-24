//! The small part of Mach-O the linker needs: x86_64 slice selection, segments, sections,
//! the symbol table and the dynamic symbol table. Mirrors `Macho` in `tools/kcbuild.py`.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;

pub const CPU_X86_64: u32 = 0x0100_0007;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: u32 = 0xcafe_babe;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
const LC_SEGMENT_64: u32 = 0x19;
const LC_UUID: u32 = 0x1b;
/// `n_type & N_TYPE == N_SECT`: a symbol defined in a section.
const N_SECT_MASK: u8 = 0x0e;

fn u32le(d: &[u8], o: usize) -> Result<u32> {
    let b = d.get(o..o + 4).ok_or_else(|| anyhow!("mach-o read past the end at {o:#x}"))?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64le(d: &[u8], o: usize) -> Result<u64> {
    let b = d.get(o..o + 8).ok_or_else(|| anyhow!("mach-o read past the end at {o:#x}"))?;
    Ok(u64::from_le_bytes(b.try_into().unwrap()))
}

fn u32be(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().unwrap()))
}

fn name16(d: &[u8], o: usize) -> Result<String> {
    let b = d.get(o..o + 16).ok_or_else(|| anyhow!("mach-o name past the end at {o:#x}"))?;
    let end = b.iter().position(|&c| c == 0).unwrap_or(16);
    String::from_utf8(b[..end].to_vec()).context("segment or section name is not utf-8")
}

/// The x86_64 slice of a fat file, or the file itself when it is a thin x86_64 Mach-O.
pub fn x86_64_slice(data: &[u8]) -> Option<Vec<u8>> {
    if u32be(data, 0) == Some(FAT_MAGIC) {
        let n = u32be(data, 4)? as usize;
        for i in 0..n {
            let at = 8 + 20 * i;
            if u32be(data, at)? == CPU_X86_64 {
                let off = u32be(data, at + 8)? as usize;
                let size = u32be(data, at + 12)? as usize;
                return data.get(off..off.checked_add(size)?).map(<[u8]>::to_vec);
            }
        }
        return None;
    }
    if data.len() >= 8 && u32le(data, 0).ok()? == MH_MAGIC_64 && u32le(data, 4).ok()? == CPU_X86_64 {
        return Some(data.to_vec());
    }
    None
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub name: String,
    pub vm: u64,
    pub vs: u64,
    pub fo: u64,
    pub fs: u64,
    /// Offset of the load command in the file.
    pub lc: usize,
    pub nsects: u32,
}

#[derive(Debug, Clone)]
pub struct Section {
    /// Name of the segment command the section was listed under.
    pub seg: String,
    pub name: String,
    pub addr: u64,
    pub size: u64,
    /// Offset of the section header in the file.
    pub lc: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub n_type: u8,
    pub n_desc: u16,
    pub value: u64,
}

pub struct Macho {
    pub data: Vec<u8>,
    pub ncmds: u32,
    pub segs: Vec<Segment>,
    pub sects: Vec<Section>,
    /// symoff, nsyms, stroff, strsize
    pub symtab: Option<[u32; 4]>,
    /// The 18 fields of `dysymtab_command` after cmd and cmdsize.
    pub dysymtab: Option<[u32; 18]>,
}

impl Macho {
    pub fn parse(data: Vec<u8>) -> Result<Macho> {
        if u32le(&data, 0)? != MH_MAGIC_64 {
            bail!("not a 64-bit mach-o");
        }
        let ncmds = u32le(&data, 16)?;
        let mut segs = Vec::new();
        let mut sects = Vec::new();
        let mut symtab = None;
        let mut dysymtab = None;
        let mut off = 32usize;
        for _ in 0..ncmds {
            let cmd = u32le(&data, off)?;
            let size = u32le(&data, off + 4)? as usize;
            if size < 8 {
                bail!("load command at {off:#x} has size {size}");
            }
            match cmd {
                LC_SEGMENT_64 => {
                    let name = name16(&data, off + 8)?;
                    let nsects = u32le(&data, off + 64)?;
                    segs.push(Segment {
                        name: name.clone(),
                        vm: u64le(&data, off + 24)?,
                        vs: u64le(&data, off + 32)?,
                        fo: u64le(&data, off + 40)?,
                        fs: u64le(&data, off + 48)?,
                        lc: off,
                        nsects,
                    });
                    let mut so = off + 72;
                    for _ in 0..nsects {
                        sects.push(Section {
                            seg: name.clone(),
                            name: name16(&data, so)?,
                            addr: u64le(&data, so + 32)?,
                            size: u64le(&data, so + 40)?,
                            lc: so,
                        });
                        so += 80;
                    }
                }
                LC_SYMTAB => {
                    let mut t = [0u32; 4];
                    for (i, v) in t.iter_mut().enumerate() {
                        *v = u32le(&data, off + 8 + 4 * i)?;
                    }
                    symtab = Some(t);
                }
                LC_DYSYMTAB => {
                    let mut t = [0u32; 18];
                    for (i, v) in t.iter_mut().enumerate() {
                        *v = u32le(&data, off + 8 + 4 * i)?;
                    }
                    dysymtab = Some(t);
                }
                _ => {}
            }
            off += size;
        }
        Ok(Macho { data, ncmds, segs, sects, symtab, dysymtab })
    }

    /// Symbol `i` of the symbol table. The name runs to the next NUL, as in the Python.
    pub fn sym_at(&self, i: u32) -> Result<Symbol> {
        let [symoff, _, stroff, _] = self.symtab.ok_or_else(|| anyhow!("no LC_SYMTAB"))?;
        let at = symoff as usize + 16 * i as usize;
        let strx = u32le(&self.data, at)? as usize;
        let n_type = *self.data.get(at + 4).ok_or_else(|| anyhow!("symbol {i} past the end"))?;
        let n_desc = u16::from_le_bytes(
            self.data.get(at + 6..at + 8).ok_or_else(|| anyhow!("symbol {i} past the end"))?.try_into().unwrap(),
        );
        let value = u64le(&self.data, at + 8)?;
        let start = stroff as usize + strx;
        let tail = self.data.get(start..).ok_or_else(|| anyhow!("symbol {i} name past the end"))?;
        let len = tail.iter().position(|&c| c == 0).ok_or_else(|| anyhow!("symbol {i} name has no NUL"))?;
        let name = String::from_utf8(tail[..len].to_vec()).with_context(|| format!("symbol {i} name is not utf-8"))?;
        Ok(Symbol { name, n_type, n_desc, value })
    }

    fn nsyms(&self) -> u32 {
        self.symtab.map(|t| t[1]).unwrap_or(0)
    }

    /// Defined external symbols (the dysymtab extdef range), in table order.
    pub fn exports(&self) -> Result<Vec<(String, u64)>> {
        let Some(dy) = self.dysymtab else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for i in dy[2]..dy[2] + dy[3] {
            let s = self.sym_at(i)?;
            if s.n_type & N_SECT_MASK == N_SECT_MASK {
                out.push((s.name, s.value));
            }
        }
        Ok(out)
    }

    /// Value of the first section-defined symbol with this name.
    pub fn find_sym(&self, name: &str) -> Result<Option<u64>> {
        for i in 0..self.nsyms() {
            let s = self.sym_at(i)?;
            if s.name == name && s.n_type & N_SECT_MASK == N_SECT_MASK {
                return Ok(Some(s.value));
            }
        }
        Ok(None)
    }

    pub fn uuid(&self) -> Option<[u8; 16]> {
        let mut off = 32usize;
        for _ in 0..self.ncmds {
            let cmd = u32le(&self.data, off).ok()?;
            let size = u32le(&self.data, off + 4).ok()? as usize;
            if cmd == LC_UUID {
                return self.data.get(off + 8..off + 24)?.try_into().ok();
            }
            if size < 8 {
                return None;
            }
            off += size;
        }
        None
    }
}

/// Every section-defined symbol of the kernel: the first local of a name wins, and an
/// external definition replaces any local one.
pub fn kernel_symbols(m: &Macho) -> Result<HashMap<String, u64>> {
    let mut locals: HashMap<String, u64> = HashMap::new();
    let mut externals: HashMap<String, u64> = HashMap::new();
    for i in 0..m.nsyms() {
        let s = m.sym_at(i)?;
        if s.n_type & N_SECT_MASK == N_SECT_MASK {
            if s.n_type & 1 != 0 {
                externals.insert(s.name, s.value);
            } else {
                locals.entry(s.name).or_insert(s.value);
            }
        }
    }
    locals.extend(externals);
    Ok(locals)
}
