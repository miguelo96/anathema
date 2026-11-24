//! The linker itself: a port of `main()` in tools/kcbuild.py. Output layout (xkc.bin):
//! eight u64 (magic, base va, kext bytes, plist offset, plist length, corecrypto start va,
//! corecrypto kmod_info va, blob length), then the blob: the linked kexts, the prelink info
//! plist at the next page, zero fill up to `PLIST_RESERVE`, and two spare pages the loader uses
//! for its EFI runtime stub and AMFI stand-in.

use crate::bundles::{find_bundles, read_info, rel, Bundle};
use crate::macho::{kernel_symbols, x86_64_slice, Macho};
use crate::plistxml::document;
use anathema_media::plist::Value;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;

pub const MAGIC: u64 = 0x434b_5841_4e41;
pub const PAGE: u64 = 0x4000;
pub const PLIST_RESERVE: u64 = 0x80000;
pub const SPARE: u64 = 0x2000;
const MH_DYLIB_IN_CACHE: u32 = 0x8000_0000;

/// The options of kcbuild.py, one field per flag.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub kernel: PathBuf,
    pub ext: Vec<PathBuf>,
    /// (bundle id, path to the .kext to use instead)
    pub overrides: Vec<(String, PathBuf)>,
    pub base: u64,
    pub roots: Vec<String>,
    pub no_start: Vec<String>,
    /// (bundle id, key)
    pub strip: Vec<(String, String)>,
    /// (bundle id, key)
    pub set_true: Vec<(String, String)>,
    /// (old id, new id)
    pub rename: Vec<(String, String)>,
    /// (bundle id, library id, version)
    pub add_lib: Vec<(String, String, String)>,
    pub codeless: Vec<String>,
    pub prefer_last: Vec<String>,
}

#[derive(Debug)]
pub struct Output {
    pub xkc: Vec<u8>,
    pub map: String,
    pub report: Report,
}

/// What kcbuild.py prints, line by line, in its order.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub lines: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for l in &self.lines {
            writeln!(f, "{l}")?;
        }
        Ok(())
    }
}

struct Exe {
    path: PathBuf,
    m: Macho,
    load: u64,
    size: u64,
    img: Vec<u8>,
    kmod: u64,
}

struct Entry {
    bid: String,
    bundle: usize,
    exe: Option<Exe>,
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Int(i)) => *i != 0,
        Some(Value::Real(r)) => *r != 0.0,
        Some(Value::Str(s)) | Some(Value::Date(s)) => !s.is_empty(),
        Some(Value::Data(d)) => !d.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Dict(d)) => !d.is_empty(),
    }
}

fn py_str_list(items: &[&String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| if s.contains('\'') { format!("\"{s}\"") } else { format!("'{s}'") }).collect();
    format!("[{}]", parts.join(", "))
}

fn dict_mut(v: &mut Value) -> Result<&mut Vec<(String, Value)>> {
    match v {
        Value::Dict(d) => Ok(d),
        _ => bail!("Info.plist is not a dictionary"),
    }
}

fn dict_set(d: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match d.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => d.push((key.to_string(), value)),
    }
}

fn dict_remove(d: &mut Vec<(String, Value)>, key: &str) {
    d.retain(|(k, _)| k != key);
}

fn put64(img: &mut [u8], at: u64, v: u64, what: &str) -> Result<()> {
    let at = at as usize;
    img.get_mut(at..at + 8).ok_or_else(|| anyhow!("{what} at {at:#x} is outside the image"))?.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn get64(img: &[u8], at: u64, what: &str) -> Result<u64> {
    let at = at as usize;
    Ok(u64::from_le_bytes(img.get(at..at + 8).ok_or_else(|| anyhow!("{what} at {at:#x} is outside the image"))?.try_into().unwrap()))
}

pub fn link(cfg: &Config) -> Result<Output> {
    let kdata = std::fs::read(&cfg.kernel).with_context(|| format!("reading {}", cfg.kernel.display()))?;
    let kslice = x86_64_slice(&kdata).ok_or_else(|| anyhow!("{} has no x86_64 kernel", cfg.kernel.display()))?;
    let ksyms = kernel_symbols(&Macho::parse(kslice)?)?;

    let mut bundles: Vec<(String, Bundle)> = find_bundles(&cfg.ext);
    let replace = |bundles: &mut Vec<(String, Bundle)>, bid: &str, b: Bundle| match bundles.iter_mut().find(|(k, _)| k == bid) {
        Some(slot) => slot.1 = b,
        None => bundles.push((bid.to_string(), b)),
    };
    if !cfg.prefer_last.is_empty() {
        let last = find_bundles(&cfg.ext[cfg.ext.len() - 1..]);
        for bid in &cfg.prefer_last {
            if let Some((_, b)) = last.iter().find(|(k, _)| k == bid) {
                replace(&mut bundles, bid, b.clone());
            }
        }
    }
    for (bid, path) in &cfg.overrides {
        let plist = path.join("Contents").join("Info.plist");
        let info = read_info(&plist)?;
        let root = path.parent().map(PathBuf::from).unwrap_or_default();
        replace(&mut bundles, bid, Bundle { path: path.clone(), plist, info, root });
    }
    let index: HashMap<String, usize> = bundles.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect();

    let libs_of = |bid: &str| -> Vec<String> {
        let info = &bundles[index[bid]].1.info;
        let libs = info.get("OSBundleLibraries_x86_64").or_else(|| info.get("OSBundleLibraries"));
        let mut out: Vec<String> = libs.and_then(Value::as_dict).map(|d| d.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
        for (ab, lib, _) in &cfg.add_lib {
            if ab == bid && !out.contains(lib) {
                out.push(lib.clone());
            }
        }
        out
    };

    // selection: closure over OSBundleLibraries, plus every pseudo kext (OSKernelResource)
    fn want(bid: &str, index: &HashMap<String, usize>, libs_of: &dyn Fn(&str) -> Vec<String>, chosen: &mut HashSet<String>, order: &mut Vec<String>) {
        if chosen.contains(bid) || !index.contains_key(bid) {
            return;
        }
        chosen.insert(bid.to_string());
        for lib in libs_of(bid) {
            want(&lib, index, libs_of, chosen, order);
        }
        order.push(bid.to_string());
    }
    let mut chosen = HashSet::new();
    let mut order = Vec::new();
    for r in &cfg.roots {
        want(r, &index, &libs_of, &mut chosen, &mut order);
    }
    for (bid, b) in &bundles {
        if matches!(b.info.get("OSKernelResource"), Some(Value::Bool(true))) {
            want(bid, &index, &libs_of, &mut chosen, &mut order);
        }
    }

    // link
    let mut report = Report::default();
    let mut entries: Vec<Entry> = Vec::new();
    let mut exports_by: Vec<(String, HashMap<String, u64>)> = Vec::new();
    let mut cur = cfg.base;
    for bid in &order {
        let bi = index[bid];
        let b = &bundles[bi].1;
        let mut exe = b.info.get("CFBundleExecutable").and_then(Value::as_str).filter(|s| !s.is_empty());
        if cfg.codeless.contains(bid) {
            exe = None;
        }
        let mut ep = None;
        if let Some(name) = exe {
            if !truthy(b.info.get("OSKernelResource")) {
                for cand in [b.path.join("Contents").join("MacOS").join(name), b.path.join(name)] {
                    if cand.exists() {
                        ep = Some(cand);
                    }
                }
            }
        }
        let Some(ep) = ep else {
            entries.push(Entry { bid: bid.clone(), bundle: bi, exe: None });
            continue;
        };
        let raw = std::fs::read(&ep).with_context(|| format!("reading {}", ep.display()))?;
        let Some(s) = x86_64_slice(&raw) else {
            report.lines.push(format!("no x86_64 slice: {bid}"));
            entries.push(Entry { bid: bid.clone(), bundle: bi, exe: None });
            continue;
        };
        let m = Macho::parse(s.clone()).with_context(|| format!("parsing {}", ep.display()))?;
        let end = m.segs.iter().filter(|sg| sg.name != "__LINKEDIT").map(|sg| sg.vm + sg.vs).max();
        let size = (end.ok_or_else(|| anyhow!("{bid}: no segments"))? + PAGE - 1) & !(PAGE - 1);
        let mut img = vec![0u8; size as usize];
        for sg in m.segs.iter().filter(|sg| sg.name != "__LINKEDIT") {
            let dst = img.get_mut(sg.vm as usize..(sg.vm + sg.fs) as usize).ok_or_else(|| anyhow!("{bid}: segment {} outside the image", sg.name))?;
            let src = s.get(sg.fo as usize..(sg.fo + sg.fs) as usize).ok_or_else(|| anyhow!("{bid}: segment {} outside the file", sg.name))?;
            dst.copy_from_slice(src);
        }
        let mut ex = HashMap::new();
        for (n, v) in m.exports()? {
            ex.insert(n, v + cur);
        }
        exports_by.push((bid.clone(), ex));
        entries.push(Entry { bid: bid.clone(), bundle: bi, exe: Some(Exe { path: ep, m, load: cur, size, img, kmod: 0 }) });
        cur += size;
    }
    let exports_index: HashMap<&str, usize> = exports_by.iter().enumerate().map(|(i, (k, _))| (k.as_str(), i)).collect();

    let deps_closure = |bid: &str| -> Vec<String> {
        fn go(bid: &str, seen: &mut Vec<String>, index: &HashMap<String, usize>, libs_of: &dyn Fn(&str) -> Vec<String>) {
            for lib in libs_of(bid) {
                if index.contains_key(&lib) && !seen.contains(&lib) {
                    seen.push(lib.clone());
                    go(&lib, seen, index, libs_of);
                }
            }
        }
        let mut seen = Vec::new();
        go(bid, &mut seen, &index, &libs_of);
        seen
    };

    // the stand-in amfi stub (xor eax,eax; ret) sits at spare page 2 + 0x800, a fixed distance
    // past the kext block because the plist length is not known yet
    let stub_va = cur + PLIST_RESERVE + 0x1000 + 0x800;
    let mut missing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let (mut ext64, mut branch, mut rebase) = (0u64, 0u64, 0u64);
    for e in entries.iter_mut() {
        let Some(x) = e.exe.as_mut() else { continue };
        let mut search = vec![e.bid.clone()];
        search.extend(deps_closure(&e.bid));
        let dy = x.m.dysymtab.ok_or_else(|| anyhow!("{}: no LC_DYSYMTAB", e.bid))?;
        let mut cache: HashMap<String, u64> = HashMap::new();
        let mut resolve = |m: &Macho, i: u32| -> Result<u64> {
            let name = m.sym_at(i)?.name;
            if let Some(v) = cache.get(&name) {
                return Ok(*v);
            }
            let addr = search
                .iter()
                .find_map(|bid| exports_index.get(bid.as_str()).and_then(|&j| exports_by[j].1.get(&name).copied()))
                .or_else(|| ksyms.get(&name).copied())
                .or_else(|| exports_by.iter().find_map(|(_, ex)| ex.get(&name).copied()));
            let addr = match addr {
                Some(a) => a,
                None => {
                    missing.entry(name.clone()).or_default().insert(e.bid.clone());
                    stub_va
                }
            };
            cache.insert(name, addr);
            Ok(addr)
        };
        let (extoff, nextrel, locoff, nlocrel) = (dy[14] as usize, dy[15] as usize, dy[16] as usize, dy[17] as usize);
        for i in 0..nextrel {
            let at = extoff + 8 * i;
            let rec = x.m.data.get(at..at + 8).ok_or_else(|| anyhow!("{}: relocation {i} outside the file", e.bid))?;
            let ra = i32::from_le_bytes(rec[..4].try_into().unwrap());
            let w = u32::from_le_bytes(rec[4..].try_into().unwrap());
            let (sn, ty, ln, pc) = (w & 0xff_ffff, (w >> 28) & 0xf, (w >> 25) & 3, (w >> 24) & 1);
            let s = resolve(&x.m, sn)?;
            if ra < 0 {
                bail!("{}: negative relocation address {ra}", e.bid);
            }
            let ra = ra as u64;
            if ty == 0 && ln == 3 && pc == 0 {
                let add = get64(&x.img, ra, "relocation")?;
                put64(&mut x.img, ra, s.wrapping_add(add), "relocation")?;
                ext64 += 1;
            } else if ty == 2 && ln == 2 && pc == 1 {
                let site = x.img.get_mut(ra as usize..ra as usize + 4).ok_or_else(|| anyhow!("{}: branch at {ra:#x} outside the image", e.bid))?;
                let add = i32::from_le_bytes(site[..].try_into().unwrap()) as i128;
                let p = (x.load + ra) as i128;
                let v = if s != 0 { s as i128 + add - (p + 4) } else { 0 };
                let v = i32::try_from(v).map_err(|_| anyhow!("{}: branch at {ra:#x} out of range", e.bid))?;
                site.copy_from_slice(&v.to_le_bytes());
                branch += 1;
            } else {
                bail!("unhandled ext reloc ({ty}, {ln}, {pc}) in {}", e.bid);
            }
        }
        for i in 0..nlocrel {
            let at = locoff + 8 * i;
            let rec = x.m.data.get(at..at + 8).ok_or_else(|| anyhow!("{}: local relocation {i} outside the file", e.bid))?;
            let ra = i32::from_le_bytes(rec[..4].try_into().unwrap());
            let w = u32::from_le_bytes(rec[4..].try_into().unwrap());
            let (ty, ln, pc) = ((w >> 28) & 0xf, (w >> 25) & 3, (w >> 24) & 1);
            if !(ty == 0 && ln == 3 && pc == 0) || ra < 0 {
                bail!("unhandled local reloc in {}", e.bid);
            }
            let v = get64(&x.img, ra as u64, "local relocation")?;
            put64(&mut x.img, ra as u64, v.wrapping_add(x.load), "local relocation")?;
            rebase += 1;
        }

        // header: segment and section addresses, drop linkedit, mark in-cache
        let (load, size) = (x.load, x.size);
        for sg in &x.m.segs {
            let lc = sg.lc as u64;
            if sg.name == "__LINKEDIT" {
                put64(&mut x.img, lc + 24, load + size, "__LINKEDIT command")?;
                for k in 1..4 {
                    put64(&mut x.img, lc + 24 + 8 * k, 0, "__LINKEDIT command")?;
                }
                continue;
            }
            let vm = sg.vm.checked_add(load).ok_or_else(|| anyhow!("{}: segment address overflows", e.bid))?;
            put64(&mut x.img, lc + 24, vm, "segment command")?;
        }
        for sc in x.m.sects.iter().filter(|sc| sc.seg != "__LINKEDIT") {
            let addr = sc.addr.checked_add(load).ok_or_else(|| anyhow!("{}: section address overflows", e.bid))?;
            put64(&mut x.img, sc.lc as u64 + 32, addr, "section header")?;
        }
        let flags = x.img.get_mut(24..28).ok_or_else(|| anyhow!("{}: header outside the image", e.bid))?;
        let fl = u32::from_le_bytes(flags[..].try_into().unwrap()) | MH_DYLIB_IN_CACHE;
        flags.copy_from_slice(&fl.to_le_bytes());
        if let Some(kv) = x.m.find_sym("_kmod_info")? {
            x.kmod = kv + load;
            let hdr_size = 32 + u32::from_le_bytes(x.m.data[20..24].try_into().unwrap()) as u64;
            put64(&mut x.img, kv + 156, load, "kmod_info")?;
            put64(&mut x.img, kv + 164, size, "kmod_info")?;
            put64(&mut x.img, kv + 172, hdr_size, "kmod_info")?;
            if cfg.no_start.contains(&e.bid) {
                put64(&mut x.img, kv + 180, 0, "kmod_info")?;
                put64(&mut x.img, kv + 188, 0, "kmod_info")?;
            }
        }
    }

    for e in &entries {
        if let Some(x) = &e.exe {
            if x.kmod == 0 {
                report.lines.push(format!("NO _kmod_info symbol: {} {}", e.bid, rel(&x.path, &bundles[e.bundle].1.root)));
            }
        }
    }

    // assemble block and plist
    let total = cur - cfg.base;
    let mut block = vec![0u8; total as usize];
    for x in entries.iter().filter_map(|e| e.exe.as_ref()) {
        let off = (x.load - cfg.base) as usize;
        block[off..off + x.size as usize].copy_from_slice(&x.img);
    }

    let mut infos = Vec::with_capacity(entries.len());
    for e in &entries {
        let b = &bundles[e.bundle].1;
        let mut info = b.info.clone();
        let d = dict_mut(&mut info).with_context(|| b.plist.display().to_string())?;
        if cfg.codeless.contains(&e.bid) {
            dict_remove(d, "CFBundleExecutable");
            dict_remove(d, "IOKitPersonalities");
        }
        for (sb, sk) in &cfg.strip {
            if *sb == e.bid {
                dict_remove(d, sk);
            }
        }
        for (ab, lib, ver) in &cfg.add_lib {
            if *ab == e.bid {
                let mut any = false;
                for key in ["OSBundleLibraries", "OSBundleLibraries_x86_64"] {
                    if let Some((_, v)) = d.iter_mut().find(|(k, _)| k == key) {
                        any = true;
                        let libs = dict_mut(v).with_context(|| format!("{key} in {}", e.bid))?;
                        dict_set(libs, lib, Value::Str(ver.clone()));
                    }
                }
                if !any {
                    dict_set(d, "OSBundleLibraries", Value::Dict(vec![(lib.clone(), Value::Str(ver.clone()))]));
                }
            }
        }
        for (old, new) in &cfg.rename {
            if *old == e.bid {
                dict_set(d, "CFBundleIdentifier", Value::Str(new.clone()));
            }
        }
        for (sb, sk) in &cfg.set_true {
            if *sb == e.bid {
                dict_set(d, sk, Value::Bool(true));
            }
        }
        dict_set(d, "_PrelinkBundlePath", Value::Str(format!("/System/Library/Extensions/{}", rel(&b.path, &b.root))));
        if let Some(x) = &e.exe {
            dict_set(d, "_PrelinkExecutableRelativePath", Value::Str(rel(&x.path, &b.path)));
            dict_set(d, "_PrelinkExecutableLoadAddr", Value::Int(x.load as i64));
            dict_set(d, "_PrelinkExecutableSize", Value::Int(x.size as i64));
            dict_set(d, "_PrelinkKmodInfo", Value::Int(x.kmod as i64));
        }
        infos.push(info);
    }
    let plist = document(&infos);
    let plist_off = (total + PAGE - 1) & !(PAGE - 1);
    if plist.len() as u64 > PLIST_RESERVE {
        bail!("plist larger than the reserved {PLIST_RESERVE:#x} bytes");
    }
    let mut blob = vec![0u8; plist_off as usize];
    blob[..total as usize].copy_from_slice(&block);
    blob.extend_from_slice(&plist);
    blob.resize(blob.len() + (PLIST_RESERVE as usize - plist.len()) + SPARE as usize, 0);

    let (mut cc_start, mut cc_kmod) = (0u64, 0u64);
    if let Some(x) = entries.iter().find(|e| e.bid == "com.apple.kec.corecrypto").and_then(|e| e.exe.as_ref()) {
        let start = x.m.find_sym("_corecrypto_kext_start")?.ok_or_else(|| anyhow!("corecrypto has no _corecrypto_kext_start"))?;
        cc_start = x.load + start;
        cc_kmod = x.kmod;
    }
    let mut xkc = Vec::with_capacity(64 + blob.len());
    for v in [MAGIC, cfg.base, total, plist_off, plist.len() as u64, cc_start, cc_kmod, blob.len() as u64] {
        xkc.extend_from_slice(&v.to_le_bytes());
    }
    xkc.extend_from_slice(&blob);

    let mut map = String::new();
    for e in &entries {
        if let Some(x) = &e.exe {
            map.push_str(&format!("{} {:#x} {:#x} {}\n", e.bid, x.load, x.size, x.path.display()));
        }
    }
    let with_code = entries.iter().filter(|e| e.exe.is_some()).count();
    report.lines.push(format!("kexts selected: {} (with code: {with_code})", entries.len()));
    report.lines.push(format!(
        "block {total:#x} bytes, plist {} bytes, relocs: {{'ext64': {ext64}, 'branch': {branch}, 'rebase': {rebase}}}",
        plist.len()
    ));
    report.lines.push(format!("corecrypto start {cc_start:#x} kmod {cc_kmod:#x}"));
    if !missing.is_empty() {
        report.lines.push(format!("UNRESOLVED symbols ({}):", missing.len()));
        for (name, who) in missing.iter().take(40) {
            let who: Vec<&String> = who.iter().take(3).collect();
            report.lines.push(format!("   {name} {}", py_str_list(&who)));
        }
    }
    Ok(Output { xkc, map, report })
}
