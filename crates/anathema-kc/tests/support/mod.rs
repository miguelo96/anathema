//! Builds small synthetic x86_64 kext Mach-O files for tests. No Apple binaries are involved.
#![allow(dead_code)]

pub const CPU_X86_64: u32 = 0x0100_0007;
pub const CPU_ARM64: u32 = 0x0100_000c;
pub const TEXT_AT: u64 = 0x1000;
pub const DATA_VM: u64 = 0x4000;
const PAGE: u64 = 0x4000;

/// One synthetic kext.
///
/// Layout: `__TEXT` at vm 0 (the Mach-O header lives at its start, `text` at `TEXT_AT`),
/// `__DATA` at `DATA_VM` holding `data`, then `__LINKEDIT` with symbols, strings and relocations.
/// Symbol numbers: exports come first (in order), then `_kmod_info` if `kmod_info_at` is set,
/// then imports.
#[derive(Default, Clone)]
pub struct KextSpec {
    pub name: &'static str,
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    pub exports: Vec<(&'static str, u64)>,
    pub locals: Vec<(&'static str, u64)>,
    pub imports: Vec<&'static str>,
    /// (r_address, symbolnum, type, length, pcrel)
    pub ext_relocs: Vec<(i32, u32, u8, u8, bool)>,
    /// r_address of each 64-bit local relocation
    pub local_relocs: Vec<i32>,
    pub kmod_info_at: Option<u64>,
    pub uuid: Option<[u8; 16]>,
}

impl KextSpec {
    /// Symbol number of an import, for use in `ext_relocs`.
    pub fn import_index(&self, name: &str) -> u32 {
        let base = self.exports.len() + usize::from(self.kmod_info_at.is_some()) + self.locals.len();
        (base + self.imports.iter().position(|n| *n == name).expect("import")) as u32
    }
    /// Size the linker will give this kext (all segments but `__LINKEDIT`, page aligned).
    pub fn linked_size(&self) -> u64 {
        DATA_VM + data_vmsize(&self.data)
    }
}

fn data_vmsize(data: &[u8]) -> u64 {
    ((data.len() as u64).max(1) + PAGE - 1) & !(PAGE - 1)
}

fn p32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn p64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn name16(b: &mut Vec<u8>, s: &str) {
    let mut n = [0u8; 16];
    n[..s.len()].copy_from_slice(s.as_bytes());
    b.extend_from_slice(&n);
}

fn segment(cmds: &mut Vec<u8>, name: &str, vm: u64, vs: u64, fo: u64, fs: u64, sects: &[(&str, u64, u64, u32)]) {
    p32(cmds, 0x19);
    p32(cmds, 72 + 80 * sects.len() as u32);
    name16(cmds, name);
    p64(cmds, vm);
    p64(cmds, vs);
    p64(cmds, fo);
    p64(cmds, fs);
    p32(cmds, 7);
    p32(cmds, 7);
    p32(cmds, sects.len() as u32);
    p32(cmds, 0);
    for (sname, addr, size, off) in sects {
        name16(cmds, sname);
        name16(cmds, name);
        p64(cmds, *addr);
        p64(cmds, *size);
        p32(cmds, *off);
        cmds.extend_from_slice(&[0u8; 28]);
    }
}

pub fn build_kext(spec: &KextSpec) -> Vec<u8> {
    let data_vs = data_vmsize(&spec.data);
    let linkedit_off = DATA_VM + data_vs;

    // symbols: exports (+ _kmod_info), locals, imports
    let mut syms: Vec<(String, u8, u8, u64)> = Vec::new();
    for (n, v) in &spec.exports {
        syms.push((n.to_string(), 0x0f, 1, *v));
    }
    if let Some(k) = spec.kmod_info_at {
        syms.push(("_kmod_info".into(), 0x0f, 2, k));
    }
    let nextdef = syms.len() as u32;
    for (n, v) in &spec.locals {
        syms.push((n.to_string(), 0x0e, 1, *v));
    }
    let nlocal = spec.locals.len() as u32;
    for n in &spec.imports {
        syms.push((n.to_string(), 0x01, 0, 0));
    }
    let mut strtab = vec![0u8];
    let mut nlist = Vec::new();
    for (n, t, s, v) in &syms {
        p32(&mut nlist, strtab.len() as u32);
        nlist.push(*t);
        nlist.push(*s);
        nlist.extend_from_slice(&0u16.to_le_bytes());
        p64(&mut nlist, *v);
        strtab.extend_from_slice(n.as_bytes());
        strtab.push(0);
    }
    let mut ext = Vec::new();
    for (addr, sym, ty, len, pc) in &spec.ext_relocs {
        ext.extend_from_slice(&addr.to_le_bytes());
        let w = (sym & 0xff_ffff) | ((*pc as u32) << 24) | ((*len as u32) << 25) | (1 << 27) | ((*ty as u32) << 28);
        p32(&mut ext, w);
    }
    let mut loc = Vec::new();
    for addr in &spec.local_relocs {
        loc.extend_from_slice(&addr.to_le_bytes());
        p32(&mut loc, 2 | (3 << 25));
    }
    let extreloff = linkedit_off;
    let locreloff = extreloff + ext.len() as u64;
    let symoff = locreloff + loc.len() as u64;
    let stroff = symoff + nlist.len() as u64;
    let linkedit_size = stroff + strtab.len() as u64 - linkedit_off;

    let mut cmds = Vec::new();
    let mut ncmds = 0;
    segment(&mut cmds, "__TEXT", 0, DATA_VM, 0, DATA_VM, &[("__text", TEXT_AT, spec.text.len() as u64, TEXT_AT as u32)]);
    ncmds += 1;
    segment(&mut cmds, "__DATA", DATA_VM, data_vs, DATA_VM, data_vs, &[("__data", DATA_VM, spec.data.len() as u64, DATA_VM as u32)]);
    ncmds += 1;
    segment(&mut cmds, "__LINKEDIT", linkedit_off, (linkedit_size + PAGE - 1) & !(PAGE - 1), linkedit_off, linkedit_size, &[]);
    ncmds += 1;
    p32(&mut cmds, 2);
    p32(&mut cmds, 24);
    p32(&mut cmds, symoff as u32);
    p32(&mut cmds, syms.len() as u32);
    p32(&mut cmds, stroff as u32);
    p32(&mut cmds, strtab.len() as u32);
    ncmds += 1;
    p32(&mut cmds, 0xb);
    p32(&mut cmds, 80);
    let nimport = spec.imports.len() as u32;
    let dy = [
        nextdef, nlocal, 0, nextdef, nextdef + nlocal, nimport, 0, 0, 0, 0, 0, 0, 0, 0,
        extreloff as u32, spec.ext_relocs.len() as u32, locreloff as u32, spec.local_relocs.len() as u32,
    ];
    for v in dy {
        p32(&mut cmds, v);
    }
    ncmds += 1;
    if let Some(u) = spec.uuid {
        p32(&mut cmds, 0x1b);
        p32(&mut cmds, 24);
        cmds.extend_from_slice(&u);
        ncmds += 1;
    }

    let mut out = Vec::new();
    p32(&mut out, 0xfeed_facf);
    p32(&mut out, CPU_X86_64);
    p32(&mut out, 3);
    p32(&mut out, 0xb);
    p32(&mut out, ncmds);
    p32(&mut out, cmds.len() as u32);
    p32(&mut out, 0x85);
    p32(&mut out, 0);
    out.extend_from_slice(&cmds);
    assert!(out.len() as u64 <= TEXT_AT, "load commands overflow into text");
    out.resize(TEXT_AT as usize, 0);
    out.extend_from_slice(&spec.text);
    assert!(out.len() as u64 <= DATA_VM, "text too large");
    out.resize(DATA_VM as usize, 0);
    out.extend_from_slice(&spec.data);
    out.resize(linkedit_off as usize, 0);
    out.extend_from_slice(&ext);
    out.extend_from_slice(&loc);
    out.extend_from_slice(&nlist);
    out.extend_from_slice(&strtab);
    out
}

/// A big-endian fat file holding the given (cputype, slice) pairs, slices 0x1000 aligned.
pub fn fat(slices: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
    out.extend_from_slice(&(slices.len() as u32).to_be_bytes());
    let mut off = 0x1000u32;
    let mut placed = Vec::new();
    for (ct, s) in slices {
        out.extend_from_slice(&ct.to_be_bytes());
        out.extend_from_slice(&3u32.to_be_bytes());
        out.extend_from_slice(&off.to_be_bytes());
        out.extend_from_slice(&(s.len() as u32).to_be_bytes());
        out.extend_from_slice(&12u32.to_be_bytes());
        placed.push(off);
        off = (off + s.len() as u32 + 0xfff) & !0xfff;
    }
    for ((_, s), at) in slices.iter().zip(placed) {
        out.resize(at as usize, 0);
        out.extend_from_slice(s);
    }
    out
}

/// A unique empty temporary directory for one test.
pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("anathema-kc-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
