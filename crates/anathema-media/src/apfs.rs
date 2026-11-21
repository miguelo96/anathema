use crate::codec;
use crate::readat::{le16, le32, le64, ReadAt};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

const T_INODE: u64 = 3;
const T_XATTR: u64 = 4;
const T_EXTENT: u64 = 8;
const T_DREC: u64 = 9;
const T_SNAP_META: u64 = 1;
const ROOT_DIR: u64 = 2;
const UF_COMPRESSED: u32 = 0x20;
const MAX_LINKS: usize = 40;

pub struct Container {
    src: Box<dyn ReadAt>,
    pub bs: usize,
    pub xid: u64,
    omap_root: u64,
    fs_oids: Vec<u64>,
    cache: RefCell<HashMap<u64, Rc<Vec<u8>>>>,
    base: u64,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub xid: u64,
    pub name: String,
    pub sblock: u64,
}

#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub index: usize,
    pub name: String,
    pub role: u16,
    pub incompat: u64,
    pub snapshots: Vec<Snapshot>,
    pub sealed: bool,
}

pub struct Volume<'a> {
    c: &'a Container,
    pub info: VolumeInfo,
    omap_root: u64,
    xid: u64,
    root_tree: u64,
    fext: Option<(u64, bool)>,
    case_insensitive: bool,
}

#[derive(Debug, Clone)]
pub struct Inode {
    pub id: u64,
    pub parent: u64,
    pub private_id: u64,
    pub mode: u16,
    pub bsd_flags: u32,
    pub nlink: i32,
    pub mtime: u64,
    pub size: u64,
    pub uncompressed: u64,
}

impl Inode {
    pub fn is_dir(&self) -> bool {
        self.mode & 0xf000 == 0x4000
    }
    pub fn is_file(&self) -> bool {
        self.mode & 0xf000 == 0x8000
    }
    pub fn is_symlink(&self) -> bool {
        self.mode & 0xf000 == 0xa000
    }
    pub fn compressed(&self) -> bool {
        self.bsd_flags & UF_COMPRESSED != 0
    }
    pub fn logical_size(&self) -> u64 {
        if self.compressed() {
            self.uncompressed
        } else {
            self.size
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub id: u64,
    pub kind: u8,
}

impl DirEntry {
    pub fn kind_char(&self) -> char {
        match self.kind {
            4 => 'd',
            8 => '-',
            10 => 'l',
            _ => '?',
        }
    }
}

struct Node<'a> {
    d: &'a [u8],
    leaf: bool,
    fixed: bool,
    n: usize,
    toc: usize,
    kstart: usize,
    vend: usize,
    ksize: usize,
    vsize: usize,
}

impl<'a> Node<'a> {
    fn parse(d: &'a [u8], ksize: usize, vsize: usize) -> Result<Self> {
        ensure!(d.len() >= 56, "short btree node");
        let flags = le16(d, 32);
        let root = flags & 1 != 0;
        let toc_off = le16(d, 40) as usize;
        let toc_len = le16(d, 42) as usize;
        Ok(Node {
            d,
            leaf: flags & 2 != 0,
            fixed: flags & 4 != 0,
            n: le32(d, 36) as usize,
            toc: 56 + toc_off,
            kstart: 56 + toc_off + toc_len,
            vend: d.len() - if root { 40 } else { 0 },
            ksize,
            vsize,
        })
    }
    fn entry(&self, i: usize) -> Result<(&'a [u8], &'a [u8])> {
        if self.fixed {
            let p = self.toc + i * 4;
            let ko = le16(self.d, p) as usize;
            let vo = le16(self.d, p + 2) as usize;
            let k = self.kstart + ko;
            let vlen = if self.leaf { self.vsize } else { 8 };
            ensure!(k + self.ksize <= self.d.len() && vo <= self.vend, "bad fixed toc entry");
            let v = self.vend - vo;
            ensure!(v + vlen <= self.d.len(), "bad fixed value");
            Ok((&self.d[k..k + self.ksize], &self.d[v..v + vlen]))
        } else {
            let p = self.toc + i * 8;
            let ko = le16(self.d, p) as usize;
            let kl = le16(self.d, p + 2) as usize;
            let vo = le16(self.d, p + 4) as usize;
            let vl = le16(self.d, p + 6) as usize;
            let k = self.kstart + ko;
            ensure!(k + kl <= self.d.len() && vo <= self.vend, "bad toc entry");
            let v = self.vend - vo;
            ensure!(v + vl <= self.d.len(), "bad value extent");
            Ok((&self.d[k..k + kl], &self.d[v..v + vl]))
        }
    }
}

fn root_info(d: &[u8]) -> (usize, usize) {
    let t = d.len() - 40;
    (le32(d, t + 8) as usize, le32(d, t + 12) as usize)
}

impl Container {
    pub fn open(src: Box<dyn ReadAt>) -> Result<Self> {
        let probe = src.read_vec(0, 4096)?;
        let base = if &probe[32..36] == b"NXSB" {
            0
        } else {
            bail!("no NXSB at block 0");
        };
        let bs = le32(&probe, 36) as usize;
        ensure!((4096..=65536).contains(&bs) && bs.is_power_of_two(), "odd block size {bs}");
        let first = src.read_vec(0, bs)?;
        let desc_blocks = le32(&first, 104) as u64;
        let desc_base = le64(&first, 112);
        ensure!(desc_base >> 63 == 0, "non-contiguous checkpoint descriptor area");
        let mut best: Option<(u64, Vec<u8>)> = None;
        for i in 0..desc_blocks {
            let b = src.read_vec((desc_base + i) * bs as u64, bs)?;
            if &b[32..36] == b"NXSB" && le32(&b, 24) & 0xffff == 1 {
                let xid = le64(&b, 16);
                if best.as_ref().is_none_or(|(x, _)| xid > *x) {
                    best = Some((xid, b));
                }
            }
        }
        let (xid, sb) = best.unwrap_or((le64(&first, 16), first));
        let omap_oid = le64(&sb, 160);
        let omap = src.read_vec(omap_oid * bs as u64, bs)?;
        let omap_root = le64(&omap, 48);
        let max_fs = le32(&sb, 180) as usize;
        let fs_oids = (0..max_fs.min(100)).map(|i| le64(&sb, 184 + i * 8)).filter(|&o| o != 0).collect();
        Ok(Self { src, bs, xid, omap_root, fs_oids, cache: RefCell::new(HashMap::new()), base })
    }

    fn block(&self, addr: u64) -> Result<Rc<Vec<u8>>> {
        if let Some(b) = self.cache.borrow().get(&addr) {
            return Ok(b.clone());
        }
        let b = Rc::new(
            self.src
                .read_vec(self.base + addr * self.bs as u64, self.bs)
                .with_context(|| format!("reading block {addr}"))?,
        );
        let mut c = self.cache.borrow_mut();
        if c.len() > 8192 {
            c.clear();
        }
        c.insert(addr, b.clone());
        Ok(b)
    }

    fn omap_lookup(&self, tree_root: u64, oid: u64, xid: u64) -> Result<Option<u64>> {
        let mut addr = tree_root;
        loop {
            let blk = self.block(addr)?;
            let (ks, vs) = if le16(&blk, 32) & 1 != 0 { root_info(&blk) } else { (16, 16) };
            let node = Node::parse(&blk, ks, vs)?;
            let mut pick: Option<usize> = None;
            for i in 0..node.n {
                let (k, _) = node.entry(i)?;
                let (ko, kx) = (le64(k, 0), le64(k, 8));
                if (ko, kx) <= (oid, xid) {
                    pick = Some(i);
                } else {
                    break;
                }
            }
            let Some(i) = pick else { return Ok(None) };
            let (k, v) = node.entry(i)?;
            if node.leaf {
                if le64(k, 0) != oid || le32(v, 0) & 1 != 0 {
                    return Ok(None);
                }
                return Ok(Some(le64(v, 8)));
            }
            addr = le64(v, 0);
        }
    }

    pub fn volume_count(&self) -> usize {
        self.fs_oids.len()
    }

    fn read_volume_sb(&self, oid: u64) -> Result<Rc<Vec<u8>>> {
        let addr = self
            .omap_lookup(self.omap_root, oid, self.xid)?
            .with_context(|| format!("volume object {oid} is not in the container omap"))?;
        let b = self.block(addr)?;
        ensure!(&b[32..36] == b"APSB", "volume superblock magic");
        Ok(b)
    }

    pub fn volumes(&self) -> Result<Vec<VolumeInfo>> {
        (0..self.fs_oids.len()).map(|i| Ok(self.open_volume(i)?.info)).collect()
    }

    pub fn open_volume(&self, index: usize) -> Result<Volume<'_>> {
        let oid = *self.fs_oids.get(index).context("no such volume")?;
        let sb = self.read_volume_sb(oid)?;
        self.volume_from_sb(index, &sb, None)
    }

    fn volume_from_sb<'a>(&'a self, index: usize, sb: &[u8], snap: Option<&Snapshot>) -> Result<Volume<'a>> {
        let vol_omap_phys = self.block(le64(sb, 128))?;
        let omap_root = le64(&vol_omap_phys, 48);
        let incompat = le64(sb, 56);
        let name_raw = &sb[704..960];
        let name_end = name_raw.iter().position(|&b| b == 0).unwrap_or(256);
        let name = String::from_utf8_lossy(&name_raw[..name_end]).into_owned();
        let sealed = le64(sb, 1024) != 0;
        let mut snapshots = Vec::new();
        let snap_tree = le64(sb, 152);
        if snap_tree != 0 {
            snapshots = self.read_snapshots(snap_tree)?;
        }
        let info = VolumeInfo { index, name, role: le16(sb, 964), incompat, snapshots, sealed };
        let (root_tree, xid) = match snap {
            Some(s) => {
                let ssb = self.block(s.sblock)?;
                ensure!(&ssb[32..36] == b"APSB", "snapshot superblock magic");
                (le64(&ssb, 136), s.xid)
            }
            None => (le64(sb, 136), le64(sb, 16)),
        };
        let fext_oid = le64(sb, 1032);
        let fext = (fext_oid != 0).then(|| (fext_oid, le32(sb, 1040) & 0x8000_0000 != 0));
        Ok(Volume { c: self, info, omap_root, xid, root_tree, fext, case_insensitive: incompat & 3 != 0 })
    }

    fn read_snapshots(&self, tree_root: u64) -> Result<Vec<Snapshot>> {
        let mut out = Vec::new();
        self.walk_phys(tree_root, &|k: &[u8]| if le64(k, 0) >> 60 == T_SNAP_META { Ordering::Equal } else { Ordering::Less }, &mut |k, v| {
            let xid = le64(k, 0) & 0x0fff_ffff_ffff_ffff;
            let sblock = le64(v, 8);
            let nlen = le16(v, 48) as usize;
            let name = String::from_utf8_lossy(&v[50..(50 + nlen).saturating_sub(1).min(v.len())]).into_owned();
            out.push(Snapshot { xid, name, sblock });
            Ok(true)
        })?;
        out.sort_by_key(|s| s.xid);
        Ok(out)
    }

    fn walk_phys(
        &self,
        root: u64,
        want: &dyn Fn(&[u8]) -> Ordering,
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<bool>,
    ) -> Result<bool> {
        let blk = self.block(root)?;
        let (ks, vs) = root_info(&blk);
        self.walk_node(&blk, ks, vs, None, want, visit)
    }

    fn walk_node(
        &self,
        blk: &[u8],
        ks: usize,
        vs: usize,
        virt: Option<(u64, u64)>,
        want: &dyn Fn(&[u8]) -> Ordering,
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<bool>,
    ) -> Result<bool> {
        let node = Node::parse(blk, ks, vs)?;
        if node.leaf {
            for i in 0..node.n {
                let (k, v) = node.entry(i)?;
                match want(k) {
                    Ordering::Less => {}
                    Ordering::Equal => {
                        if !visit(k, v)? {
                            return Ok(false);
                        }
                    }
                    Ordering::Greater => return Ok(false),
                }
            }
            return Ok(true);
        }
        for i in 0..node.n {
            let (k, v) = node.entry(i)?;
            if want(k) == Ordering::Greater {
                return Ok(false);
            }
            if i + 1 < node.n {
                let (nk, _) = node.entry(i + 1)?;
                if want(nk) == Ordering::Less {
                    continue;
                }
            }
            let child = le64(v, 0);
            let addr = match virt {
                Some((omap, xid)) => self
                    .omap_lookup(omap, child, xid)?
                    .with_context(|| format!("tree node {child} missing from omap"))?,
                None => child,
            };
            let cb = self.block(addr)?;
            if !self.walk_node(&cb, ks, vs, virt, want, visit)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn hdr(k: &[u8]) -> (u64, u64) {
    let v = le64(k, 0);
    (v & 0x0fff_ffff_ffff_ffff, v >> 60)
}

fn utf8(b: &[u8]) -> String {
    let e = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..e]).into_owned()
}

impl<'a> Volume<'a> {
    pub fn snapshot_names(&self) -> Vec<String> {
        self.info.snapshots.iter().map(|s| format!("{} (xid {})", s.name, s.xid)).collect()
    }

    pub fn at_snapshot(&self, xid: u64) -> Result<Volume<'a>> {
        let snap = self.info.snapshots.iter().find(|s| s.xid == xid).with_context(|| format!("no snapshot with xid {xid}"))?.clone();
        let oid = *self.c.fs_oids.get(self.info.index).unwrap();
        let sb = self.c.read_volume_sb(oid)?;
        self.c.volume_from_sb(self.info.index, &sb, Some(&snap))
    }

    pub fn latest_snapshot(&self) -> Result<Option<Volume<'a>>> {
        match self.info.snapshots.last() {
            Some(s) => Ok(Some(self.at_snapshot(s.xid)?)),
            None => Ok(None),
        }
    }

    fn scan(&self, id: u64, ty: u64, visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<bool>) -> Result<()> {
        let blk = self.c.omap_lookup(self.omap_root, self.root_tree, self.xid)?.context("root tree not in omap")?;
        let b = self.c.block(blk)?;
        let (ks, vs) = root_info(&b);
        let virt = self.child_virt(&b);
        self.c.walk_node(&b, ks, vs, virt, &|k| hdr(k).cmp(&(id, ty)), visit)?;
        Ok(())
    }

    fn child_virt(&self, root: &[u8]) -> Option<(u64, u64)> {
        let flags = le32(root, root.len() - 40);
        (flags & 0x10 == 0).then_some((self.omap_root, self.xid))
    }

    pub fn first_records(&self, n: usize) -> Result<Vec<(u64, u64, usize)>> {
        let blk = self.c.omap_lookup(self.omap_root, self.root_tree, self.xid)?.context("root tree not in omap")?;
        let b = self.c.block(blk)?;
        let (ks, vs) = root_info(&b);
        let mut out = Vec::new();
        let virt = self.child_virt(&b);
        self.c.walk_node(&b, ks, vs, virt, &|_| Ordering::Equal, &mut |k, v| {
            let (i, t) = hdr(k);
            out.push((i, t, v.len()));
            Ok(out.len() < n)
        })?;
        Ok(out)
    }

    pub fn inode(&self, id: u64) -> Result<Inode> {
        let mut found = None;
        self.scan(id, T_INODE, &mut |_, v| {
            let mut size = 0;
            if v.len() > 96 {
                let n = le16(v, 92) as usize;
                let mut dp = 92 + 4 + n * 4;
                for e in 0..n {
                    let ty = v[96 + e * 4];
                    let sz = le16(v, 96 + e * 4 + 2) as usize;
                    if ty == 8 && dp + 8 <= v.len() {
                        size = le64(v, dp);
                    }
                    dp += (sz + 7) & !7;
                }
            }
            found = Some(Inode {
                id,
                parent: le64(v, 0),
                private_id: le64(v, 8),
                mtime: le64(v, 24),
                bsd_flags: le32(v, 68),
                nlink: le32(v, 56) as i32,
                mode: le16(v, 80),
                uncompressed: le64(v, 84),
                size,
            });
            Ok(false)
        })?;
        found.with_context(|| format!("inode {id} not found"))
    }

    pub fn list_dir(&self, id: u64) -> Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        self.scan(id, T_DREC, &mut |k, v| {
            let (nlen, off) = {
                let hashed = le32(k, 8) & 0x3ff;
                if 12 + hashed as usize == k.len() {
                    (hashed as usize, 12)
                } else {
                    (le16(k, 8) as usize, 10)
                }
            };
            let name = utf8(&k[off..(off + nlen).min(k.len())]);
            out.push(DirEntry { name, id: le64(v, 0), kind: (le16(v, 16) & 0xf) as u8 });
            Ok(true)
        })?;
        Ok(out)
    }

    pub fn xattr(&self, id: u64, name: &str) -> Result<Option<Vec<u8>>> {
        let mut found: Option<(u16, Vec<u8>)> = None;
        self.scan(id, T_XATTR, &mut |k, v| {
            let nlen = le16(k, 8) as usize;
            if utf8(&k[10..(10 + nlen).min(k.len())]) == name {
                found = Some((le16(v, 0), v.to_vec()));
                return Ok(false);
            }
            Ok(true)
        })?;
        let Some((flags, v)) = found else { return Ok(None) };
        let len = le16(&v, 2) as usize;
        if flags & 2 != 0 {
            return Ok(Some(v[4..4 + len].to_vec()));
        }
        ensure!(flags & 1 != 0, "xattr {name} has neither embedded nor stream data");
        let obj = le64(&v, 4);
        let size = le64(&v, 12);
        let mut out = Vec::with_capacity(size as usize);
        self.read_stream(obj, size, &mut out, &mut |_| {})?;
        Ok(Some(out))
    }

    fn extents(&self, private_id: u64) -> Result<Vec<(u64, u64, u64)>> {
        let mut ex = Vec::new();
        self.scan(private_id, T_EXTENT, &mut |k, v| {
            ex.push((le64(k, 8), le64(v, 0) & 0x00ff_ffff_ffff_ffff, le64(v, 8)));
            Ok(true)
        })?;
        if ex.is_empty() {
            if let Some((oid, virt)) = self.fext {
                let root = if virt { self.c.omap_lookup(self.omap_root, oid, self.xid)?.context("fext tree not in omap")? } else { oid };
                let b = self.c.block(root)?;
                let (ks, vs) = root_info(&b);
                let virt = if virt { self.child_virt(&b) } else { None };
                self.c.walk_node(&b, ks, vs, virt, &|k| le64(k, 0).cmp(&private_id), &mut |k, v| {
                    ex.push((le64(k, 8), le64(v, 0) & 0x00ff_ffff_ffff_ffff, le64(v, 8)));
                    Ok(true)
                })?;
            }
        }
        ex.sort_by_key(|e| e.0);
        Ok(ex)
    }

    fn read_stream(&self, private_id: u64, size: u64, out: &mut dyn Write, progress: &mut dyn FnMut(u64)) -> Result<u64> {
        let bs = self.c.bs as u64;
        let ex = self.extents(private_id)?;
        let mut pos = 0u64;
        let zeros = vec![0u8; 1 << 20];
        let mut buf = vec![0u8; 8 << 20];
        let zero_fill = |out: &mut dyn Write, mut n: u64| -> Result<()> {
            while n > 0 {
                let c = n.min(zeros.len() as u64);
                out.write_all(&zeros[..c as usize])?;
                n -= c;
            }
            Ok(())
        };
        for (logical, len, phys) in ex {
            if logical >= size {
                break;
            }
            if logical > pos {
                zero_fill(out, logical - pos)?;
                pos = logical;
            }
            let take = len.min(size - logical);
            if phys == 0 {
                zero_fill(out, take)?;
            } else {
                let mut done = 0u64;
                while done < take {
                    let n = (take - done).min(buf.len() as u64) as usize;
                    self.c
                        .src
                        .read_exact_at(self.c.base + phys * bs + done, &mut buf[..n])
                        .with_context(|| format!("reading extent at block {phys}"))?;
                    out.write_all(&buf[..n])?;
                    done += n as u64;
                    progress(pos + done);
                }
            }
            pos += take;
        }
        if pos < size {
            zero_fill(out, size - pos)?;
            pos = size;
        }
        Ok(pos)
    }

    pub fn write_file(&self, ino: &Inode, out: &mut dyn Write, progress: &mut dyn FnMut(u64)) -> Result<u64> {
        if ino.compressed() {
            if let Some(data) = self.decompress(ino)? {
                out.write_all(&data)?;
                return Ok(data.len() as u64);
            }
        }
        self.read_stream(ino.private_id, ino.size, out, progress)
    }

    pub fn read_file(&self, ino: &Inode) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(ino.logical_size() as usize);
        self.write_file(ino, &mut v, &mut |_| {})?;
        Ok(v)
    }

    pub fn readlink(&self, ino: &Inode) -> Result<String> {
        let v = self.xattr(ino.id, "com.apple.fs.symlink")?.context("symlink without target")?;
        Ok(utf8(&v))
    }

    fn decompress(&self, ino: &Inode) -> Result<Option<Vec<u8>>> {
        let Some(h) = self.xattr(ino.id, "com.apple.decmpfs")? else { return Ok(None) };
        ensure!(h.len() >= 16 && &h[..4] == b"fpmc", "bad decmpfs header");
        let kind = le32(&h, 4);
        let usize_ = le64(&h, 8) as usize;
        let inline = &h[16..];
        let rsrc = |s: &Self| -> Result<Vec<u8>> {
            s.xattr(ino.id, "com.apple.ResourceFork")?.context("compressed file without resource fork")
        };
        let out = match kind {
            3 => {
                if inline.first() == Some(&0xff) {
                    inline[1..].to_vec()
                } else {
                    zlib(inline, usize_)?
                }
            }
            7 | 11 => {
                if inline.first() == Some(&0x06) && kind == 7 {
                    inline[1..].to_vec()
                } else {
                    codec::lzvn_or_lzfse(inline, usize_)?
                }
            }
            4 => {
                let r = rsrc(self)?;
                let data_off = u32::from_be_bytes(r[0..4].try_into()?) as usize;
                let tb = data_off + 4;
                let nblocks = le32(&r, tb) as usize;
                let mut out = Vec::with_capacity(usize_);
                for i in 0..nblocks {
                    let off = le32(&r, tb + 4 + i * 8) as usize;
                    let len = le32(&r, tb + 8 + i * 8) as usize;
                    let chunk = &r[tb + off..tb + off + len];
                    let want = (usize_ - out.len()).min(65536);
                    if chunk.first() == Some(&0xff) {
                        out.extend_from_slice(&chunk[1..]);
                    } else {
                        out.extend(zlib(chunk, want)?);
                    }
                }
                out
            }
            8 | 12 => {
                let r = rsrc(self)?;
                let nblocks = usize_.div_ceil(65536);
                let mut offs = Vec::with_capacity(nblocks + 1);
                for i in 0..=nblocks {
                    offs.push(le32(&r, i * 4) as usize);
                }
                let mut out = Vec::with_capacity(usize_);
                for i in 0..nblocks {
                    let chunk = &r[offs[i]..offs[i + 1].min(r.len())];
                    let want = (usize_ - out.len()).min(65536);
                    if chunk.first() == Some(&0x06) && kind == 8 {
                        out.extend_from_slice(&chunk[1..]);
                    } else {
                        out.extend(codec::lzvn_or_lzfse(chunk, want)?);
                    }
                }
                out
            }
            other => bail!("unsupported decmpfs type {other}"),
        };
        Ok(Some(out))
    }

    pub fn lookup(&self, path: &str, follow_last: bool) -> Result<Inode> {
        let mut parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        parts.reverse();
        let mut cur = self.inode(ROOT_DIR)?;
        let mut links = 0;
        while let Some(name) = parts.pop() {
            if name == "." {
                continue;
            }
            if name == ".." {
                cur = self.inode(cur.parent)?;
                continue;
            }
            ensure!(cur.is_dir(), "{name}: parent is not a directory");
            let entries = self.list_dir(cur.id)?;
            let hit = entries
                .iter()
                .find(|e| e.name == name)
                .or_else(|| entries.iter().find(|e| e.name.to_lowercase() == name.to_lowercase() && self.case_insensitive))
                .ok_or_else(|| anyhow!("{path}: {name} not found"))?;
            let ino = self.inode(hit.id)?;
            if ino.is_symlink() && (follow_last || !parts.is_empty()) {
                links += 1;
                ensure!(links < MAX_LINKS, "too many symbolic links");
                let target = self.readlink(&ino)?;
                if target.starts_with('/') {
                    cur = self.inode(ROOT_DIR)?;
                }
                for p in target.split('/').filter(|s| !s.is_empty()).rev() {
                    parts.push(p.to_string());
                }
                continue;
            }
            cur = ino;
        }
        Ok(cur)
    }
}

fn zlib(data: &[u8], want: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(want);
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}
