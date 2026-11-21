use anathema_media::readat::{FileAt, ReadAt};
use anathema_media::{aea, apfs, download, feed, hash, http, im4p, manifest, zip};
use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

struct Args {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
}

impl Args {
    fn parse(raw: &[String]) -> Self {
        let mut pos = Vec::new();
        let mut opts = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            if let Some(name) = raw[i].strip_prefix("--") {
                let v = raw.get(i + 1).cloned().unwrap_or_default();
                opts.push((name.to_string(), v));
                i += 2;
            } else {
                pos.push(raw[i].clone());
                i += 1;
            }
        }
        Self { pos, opts }
    }
    fn opt(&self, name: &str) -> Option<&str> {
        self.opts.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn product(&self) -> &str {
        self.opt("product").unwrap_or(feed::DEFAULT_PRODUCT)
    }
}

fn open_source(s: &str) -> Result<Box<dyn ReadAt>> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(Box::new(http::Remote::open(http::client()?, s)?))
    } else {
        Ok(Box::new(FileAt::open(Path::new(s)).with_context(|| format!("opening {s}"))?))
    }
}

fn resolve_source(a: &Args) -> Result<String> {
    if let Some(s) = a.pos.first() {
        return Ok(s.clone());
    }
    let all = feed::fetch_feed()?;
    let c = feed::pick(&all, a.product(), a.opt("build")).context("no matching restore in the feed")?;
    Ok(c.url.clone())
}

fn out_dir(a: &Args) -> Result<PathBuf> {
    let explicit = a.opt("out").is_some();
    let dir = a.opt("out").map(PathBuf::from).unwrap_or_else(download::default_out);
    download::guard_out(&dir, explicit)?;
    Ok(dir)
}

fn progress(label: &'static str) -> impl FnMut(u64) {
    let mut last = std::time::Instant::now();
    move |n| {
        if last.elapsed() > std::time::Duration::from_secs(2) {
            eprint!("\r{label} {:.2} GB   ", n as f64 / 1e9);
            last = std::time::Instant::now();
        }
    }
}

fn member_bytes(z: &zip::Zip, name: &str) -> Result<Vec<u8>> {
    z.read(name)
}

fn load_manifest(z: &zip::Zip) -> Result<manifest::Manifest> {
    manifest::parse(&member_bytes(z, "BuildManifest.plist")?)
}

fn identity<'a>(m: &'a manifest::Manifest, a: &Args) -> Result<&'a manifest::Identity> {
    let class = match a.opt("class") {
        Some(c) => c.to_string(),
        None => manifest::device_class(a.product())
            .context("unknown device class for product, pass --class")?
            .to_string(),
    };
    m.select(&class, a.opt("variant"))
}

fn pick_volume<'a>(c: &'a apfs::Container, a: &Args) -> Result<apfs::Volume<'a>> {
    let index = match a.opt("volume") {
        Some(v) => v.parse::<usize>()?,
        None => {
            let vols = c.volumes()?;
            vols.iter().find(|v| v.role == 1).or(vols.first()).context("container has no volumes")?.index
        }
    };
    let live = c.open_volume(index)?;
    match a.opt("snapshot") {
        Some("live") => Ok(live),
        Some(x) => live.at_snapshot(x.parse()?),
        None => match live.latest_snapshot()? {
            Some(s) => Ok(s),
            None => Ok(live),
        },
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let a = Args::parse(&args[1.min(args.len())..]);
    match cmd {
        "list" => {
            let all = feed::fetch_feed()?;
            let list = feed::for_product(&all, a.product());
            if list.is_empty() {
                bail!("the feed has no restore for {}", a.product());
            }
            for (i, c) in list.iter().enumerate() {
                println!("{} {} {} {} sha1={}", if i == 0 { "*" } else { " " }, c.version, c.build, c.url, c.sha1);
            }
        }
        "products" => {
            let all = feed::fetch_feed()?;
            let mut p: Vec<_> = all.iter().map(|c| c.product.as_str()).collect();
            p.dedup();
            println!("{}", p.join(" "));
        }
        "resolve" => {
            let all = feed::fetch_feed()?;
            let c = feed::pick(&all, a.product(), a.opt("build")).context("no matching restore")?;
            println!("{}", c.url);
        }
        "download" => {
            let all = feed::fetch_feed()?;
            let c = feed::pick(&all, a.product(), a.opt("build")).context("no matching restore")?;
            let dir = out_dir(&a)?;
            eprintln!("{} {} {} -> {}", c.product, c.version, c.build, dir.display());
            let path = download::download(&c.url, &dir, (!c.sha1.is_empty()).then_some(c.sha1.as_str()))?;
            println!("{}", path.display());
        }
        "ls" => {
            let src = open_source(&resolve_source(&a)?)?;
            let z = zip::Zip::open(&*src)?;
            for e in &z.entries {
                println!("{:>14} {:>14} {}", e.size, e.csize, e.name);
            }
        }
        "manifest" => {
            let src = open_source(&resolve_source(&a)?)?;
            let z = zip::Zip::open(&*src)?;
            let m = load_manifest(&z)?;
            println!("build {} version {}", m.build, m.version);
            println!("products {}", m.products.join(" "));
            let id = identity(&m, &a)?;
            println!("identity class={} variant={} chip={} board={}", id.device_class, id.variant, id.chip, id.board);
            for c in &id.components {
                println!("  {:<28} {}", c.name, c.path);
            }
        }
        "im4p" => {
            let (member, out) = (a.pos.get(1).context("usage: im4p <ipsw> <member> [out]")?, a.pos.get(2));
            let src = open_source(&a.pos[0])?;
            let z = zip::Zip::open(&*src)?;
            let (im, data) = im4p::decode(&member_bytes(&z, member)?)?;
            println!("type {} desc {:?} payload {} bytes -> {} bytes compression {:?}", im.kind, im.desc, im.payload.len(), data.len(), im.compression);
            if let Ok(mo) = im4p::parse_macho(&data) {
                println!("mach-o cpu {:#x} filetype {:#x} ncmds {} fileset entries {}", mo.cpu, mo.filetype, mo.ncmds, mo.entries.len());
                for e in mo.entries.iter().take(8) {
                    println!("  {e}");
                }
            } else if let Ok(t) = im4p::parse_devicetree(&data) {
                let mut s = String::new();
                t.dump(0, &mut s);
                for l in s.lines().take(40) {
                    println!("{l}");
                }
            }
            if let Some(o) = out {
                std::fs::write(o, &data)?;
            }
        }
        "aea-info" => {
            let member = a.pos.get(1).context("usage: aea-info <ipsw> <member>")?;
            let src = open_source(&a.pos[0])?;
            let z = zip::Zip::open(&*src)?;
            let e = z.find(member).context("member not found")?.clone();
            let h = aea::read_header(&mut z.reader(&e)?)?;
            println!("profile {} size {}", h.profile, e.size);
            for (k, v) in &h.fields {
                if v.len() < 200 && v.iter().all(|b| (0x20..0x7f).contains(b)) {
                    println!("  {k} = {}", String::from_utf8_lossy(v));
                } else {
                    println!("  {k} = <{} bytes sha256 {}>", v.len(), &hash::sha256(v)[..16]);
                }
            }
        }
        "decrypt" => {
            let member = a.pos.get(1).context("usage: decrypt <ipsw> <member> --out <file>")?;
            let outp = a.opt("out").context("--out <file> is required")?;
            let src = open_source(&a.pos[0])?;
            let z = zip::Zip::open(&*src)?;
            let e = z.find(member).context("member not found")?.clone();
            let h = aea::read_header(&mut z.reader(&e)?)?;
            let pem = aea::fetch_key_pem(&h)?;
            let key = aea::archive_key(&h, &pem)?;
            let mut out = BufWriter::with_capacity(4 << 20, File::create(outp)?);
            let mut last = std::time::Instant::now();
            let n = aea::decrypt(&key, &mut z.reader(&e)?, &mut out, &mut |w, t| {
                if last.elapsed() > std::time::Duration::from_secs(2) {
                    eprint!("\rdecrypting {:5.1}%", w as f64 * 100.0 / t as f64);
                    last = std::time::Instant::now();
                }
            })?;
            out.flush()?;
            eprintln!();
            println!("{n} bytes -> {outp}");
        }
        "volumes" => {
            let img = a.pos.first().context("usage: volumes <image>")?;
            let c = apfs::Container::open(Box::new(FileAt::open(Path::new(img))?))?;
            println!("block size {} xid {} volumes {}", c.bs, c.xid, c.volume_count());
            for v in c.volumes()? {
                println!("[{}] {:?} role {:#x} incompat {:#x} sealed {} snapshots {}", v.index, v.name, v.role, v.incompat, v.sealed, v.snapshots.len());
                for s in &v.snapshots {
                    println!("      xid {} {:?}", s.xid, s.name);
                }
            }
        }
        "fsls" => {
            let img = a.pos.first().context("usage: fsls <image> [path]")?;
            let c = apfs::Container::open(Box::new(FileAt::open(Path::new(img))?))?;
            let v = pick_volume(&c, &a)?;
            let path = a.pos.get(1).map(String::as_str).unwrap_or("/");
            let ino = v.lookup(path, true)?;
            if ino.is_dir() {
                let mut e = v.list_dir(ino.id)?;
                e.sort_by(|x, y| x.name.cmp(&y.name));
                for d in e {
                    let i = v.inode(d.id)?;
                    println!("{} {:>12} {}", d.kind_char(), i.logical_size(), d.name);
                }
            } else {
                println!("{} {} bytes mode {:o}", path, ino.logical_size(), ino.mode);
            }
        }
        "extract" => {
            let img = a.pos.first().context("usage: extract <image> <path> --out <file>")?;
            let path = a.pos.get(1).context("usage: extract <image> <path> --out <file>")?;
            let outp = a.opt("out").context("--out <file> is required")?;
            let c = apfs::Container::open(Box::new(FileAt::open(Path::new(img))?))?;
            let v = pick_volume(&c, &a)?;
            let ino = v.lookup(path, true)?;
            if !ino.is_file() {
                bail!("{path} is not a regular file");
            }
            let mut out = BufWriter::with_capacity(4 << 20, File::create(outp)?);
            let n = v.write_file(&ino, &mut out, &mut progress("extracting"))?;
            out.flush()?;
            eprintln!();
            println!("{n} bytes -> {outp}");
        }
        "prepare" => {
            let src = open_source(&resolve_source(&a)?)?;
            let z = zip::Zip::open(&*src)?;
            let m = load_manifest(&z)?;
            let id = identity(&m, &a)?;
            let dir = out_dir(&a)?.join("images");
            std::fs::create_dir_all(&dir)?;
            for (comp, file) in [("KernelCache", "kernelcache.macho"), ("DeviceTree", "devicetree.bin")] {
                let path = id.path(comp).with_context(|| format!("no {comp} in identity"))?;
                let (_, data) = im4p::decode(&z.read(path)?)?;
                std::fs::write(dir.join(file), &data)?;
                println!("{comp}: {} bytes -> {}", data.len(), dir.join(file).display());
            }
            for (comp, file) in [("OS", "system.img"), ("Cryptex1,SystemOS", "cryptex-system.img"), ("Cryptex1,AppOS", "cryptex-app.img"), ("Cryptex1,RosettaOS", "cryptex-rosetta.img")] {
                let Some(path) = id.path(comp) else { continue };
                let target = dir.join(file);
                if target.exists() {
                    println!("{comp}: {} exists, skipped", target.display());
                    continue;
                }
                let e = z.find(path).with_context(|| format!("{path} missing from the archive"))?.clone();
                let part = dir.join(format!("{file}.part"));
                let mut out = BufWriter::with_capacity(4 << 20, File::create(&part)?);
                if path.ends_with(".aea") {
                    let h = aea::read_header(&mut z.reader(&e)?)?;
                    let key = aea::archive_key(&h, &aea::fetch_key_pem(&h)?)?;
                    let mut last = std::time::Instant::now();
                    aea::decrypt(&key, &mut z.reader(&e)?, &mut out, &mut |w, t| {
                        if last.elapsed() > std::time::Duration::from_secs(2) {
                            eprint!("{comp}: decrypting {:5.1}%", w as f64 * 100.0 / t as f64);
                            last = std::time::Instant::now();
                        }
                    })?;
                } else {
                    z.copy_to(&e, &mut out, &mut progress("extracting"))?;
                }
                out.flush()?;
                drop(out);
                std::fs::rename(&part, &target)?;
                eprintln!();
                println!("{comp}: -> {}", target.display());
            }
        }
        "extract-member" => {
            let member = a.pos.get(1).context("usage: extract-member <ipsw> <member> --out <file>")?;
            let outp = a.opt("out").context("--out <file> is required")?;
            let src = open_source(&a.pos[0])?;
            let z = zip::Zip::open(&*src)?;
            let e = z.find(member).context("member not found")?.clone();
            let mut out = BufWriter::new(File::create(outp)?);
            let n = z.copy_to(&e, &mut out, &mut progress("extracting"))?;
            out.flush()?;
            eprintln!();
            println!("{n} bytes -> {outp}");
        }
        _ => {
            eprintln!(
                "usage: anathema-media <command>\n  list [--product P]\n  products\n  resolve [--product P] [--build B]\n  download [--product P] [--build B] [--out DIR]\n  ls [ipsw|url]\n  manifest [ipsw|url] [--product P] [--class C] [--variant V]\n  im4p <ipsw|url> <member> [out]\n  aea-info <ipsw|url> <member>\n  decrypt <ipsw|url> <member> --out FILE\n  extract-member <ipsw|url> <member> --out FILE"
            );
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(args) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
