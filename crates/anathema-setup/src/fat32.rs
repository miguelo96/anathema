//! Builds a FAT32 file system holding a small tree of files (the EFI partition of the stick),
//! as a list of sector regions to write. One sector per cluster; every directory and file is
//! placed in consecutive clusters.

pub const SECTOR: u64 = 512;

pub struct Region {
    pub sector: u64,
    pub data: Vec<u8>,
}

/// One file to put on the partition. `path` is `/`-separated, e.g. `EFI/BOOT/BOOTX64.EFI`.
#[derive(Debug, Clone, Copy)]
pub struct FileSpec<'a> {
    pub path: &'a str,
    pub data: &'a [u8],
}

fn put16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

const ALLOWED: &[u8] = b"!#$%&'()-@^_`{}~";

/// The 8.3 directory name and the NT case flags (0x08 lower-case name, 0x10 lower-case
/// extension) that let Windows show `xkernel` while UEFI matches it case-insensitively.
fn short_name(name: &str) -> Result<([u8; 11], u8), String> {
    let bad = || format!("{name} is not an 8.3 name");
    let (base, ext) = match name.split_once('.') {
        Some((b, e)) => (b, e),
        None => (name, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || ext.contains('.') || (name.contains('.') && ext.is_empty()) {
        return Err(bad());
    }
    let mut case = 0u8;
    for (part, flag) in [(base, 0x08u8), (ext, 0x10)] {
        if !part.bytes().all(|c| c.is_ascii_alphanumeric() || ALLOWED.contains(&c)) {
            return Err(bad());
        }
        let lower = part.bytes().any(|c| c.is_ascii_lowercase());
        let upper = part.bytes().any(|c| c.is_ascii_uppercase());
        if lower && upper {
            return Err(bad());
        }
        if lower {
            case |= flag;
        }
    }
    let mut out = [b' '; 11];
    for (slot, c) in out.iter_mut().zip(base.bytes()) {
        *slot = c.to_ascii_uppercase();
    }
    for (slot, c) in out[8..].iter_mut().zip(ext.bytes()) {
        *slot = c.to_ascii_uppercase();
    }
    Ok((out, case))
}

enum Kind<'a> {
    Dir(Vec<usize>),
    File(&'a [u8]),
}

struct Node<'a> {
    short: [u8; 11],
    case: u8,
    parent: usize,
    kind: Kind<'a>,
    first: u32,
    clusters: u64,
}

fn entry(name: &[u8; 11], attr: u8, case: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    e[12] = case;
    let date: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
    put16(&mut e, 16, date);
    put16(&mut e, 24, date);
    put16(&mut e, 18, date);
    put16(&mut e, 20, (cluster >> 16) as u16);
    put16(&mut e, 26, cluster as u16);
    put32(&mut e, 28, size);
    e
}

fn tree<'a>(files: &[FileSpec<'a>]) -> Result<Vec<Node<'a>>, String> {
    let mut nodes = vec![Node { short: [b' '; 11], case: 0, parent: 0, kind: Kind::Dir(Vec::new()), first: 0, clusters: 0 }];
    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(format!("{} is not a valid path", f.path));
        }
        let mut dir = 0usize;
        for (i, part) in parts.iter().enumerate() {
            let (short, case) = short_name(part)?;
            let last = i + 1 == parts.len();
            let Kind::Dir(children) = &nodes[dir].kind else { unreachable!() };
            let found = children.iter().copied().find(|&c| nodes[c].short == short);
            match (found, last) {
                (Some(c), false) if matches!(nodes[c].kind, Kind::Dir(_)) => dir = c,
                (Some(_), true) => return Err(format!("{} is listed twice or is both a file and a folder", f.path)),
                (Some(_), false) => return Err(format!("{part} in {} is both a file and a folder", f.path)),
                (None, _) => {
                    let kind = if last { Kind::File(f.data) } else { Kind::Dir(Vec::new()) };
                    nodes.push(Node { short, case, parent: dir, kind, first: 0, clusters: 0 });
                    let id = nodes.len() - 1;
                    if let Kind::Dir(children) = &mut nodes[dir].kind {
                        children.push(id);
                    }
                    dir = id;
                }
            }
        }
    }
    Ok(nodes)
}

pub fn build(total_sectors: u64, hidden: u32, label: &str, files: &[FileSpec]) -> Result<Vec<Region>, String> {
    if total_sectors < 66_000 || total_sectors > u32::MAX as u64 {
        return Err("partition size is out of range for FAT32".into());
    }
    let reserved: u64 = 32;
    let fat_sectors = (total_sectors - reserved + 2).div_ceil(130);
    let data_start = reserved + 2 * fat_sectors;
    let clusters = total_sectors - data_start;
    if clusters < 65_525 {
        return Err("partition has too few clusters for FAT32".into());
    }

    let mut nodes = tree(files)?;
    // directories first (root first, at cluster 2), then files
    let mut next = 2u64;
    let order: Vec<usize> = (0..nodes.len())
        .filter(|&i| matches!(nodes[i].kind, Kind::Dir(_)))
        .chain((0..nodes.len()).filter(|&i| matches!(nodes[i].kind, Kind::File(_))))
        .collect();
    for &i in &order {
        let n = match &nodes[i].kind {
            Kind::Dir(children) => (((children.len() + if i == 0 { 0 } else { 2 }) * 32) as u64).div_ceil(SECTOR).max(1),
            Kind::File(data) => (data.len() as u64).div_ceil(SECTOR),
        };
        if n > 0 {
            nodes[i].first = next as u32;
            nodes[i].clusters = n;
            next += n;
        }
    }
    if next - 2 > clusters {
        let mb = |c: u64| c * SECTOR / 1_000_000;
        return Err(format!("{} MB of files does not fit in the {} MB EFI partition", mb(next - 2), mb(clusters)));
    }

    let mut boot = vec![0u8; 512];
    boot[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    boot[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut boot, 11, SECTOR as u16);
    boot[13] = 1;
    put16(&mut boot, 14, reserved as u16);
    boot[16] = 2;
    boot[21] = 0xF8;
    put16(&mut boot, 24, 63);
    put16(&mut boot, 26, 255);
    put32(&mut boot, 28, hidden);
    put32(&mut boot, 32, total_sectors as u32);
    put32(&mut boot, 36, fat_sectors as u32);
    put32(&mut boot, 44, nodes[0].first);
    put16(&mut boot, 48, 1);
    put16(&mut boot, 50, 6);
    boot[64] = 0x80;
    boot[66] = 0x29;
    let serial = (total_sectors as u32) ^ 0xA7A7_1E3A;
    put32(&mut boot, 67, serial);
    let mut name = [b' '; 11];
    for (slot, ch) in name.iter_mut().zip(label.bytes().take(11)) {
        *slot = ch.to_ascii_uppercase();
    }
    boot[71..82].copy_from_slice(&name);
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510] = 0x55;
    boot[511] = 0xAA;

    let mut info = vec![0u8; 512];
    put32(&mut info, 0, 0x4161_5252);
    put32(&mut info, 484, 0x6141_7272);
    put32(&mut info, 488, (clusters - (next - 2)) as u32);
    put32(&mut info, 492, next as u32);
    info[510] = 0x55;
    info[511] = 0xAA;

    let mut fat = vec![0u8; (fat_sectors * SECTOR) as usize];
    put32(&mut fat, 0, 0x0FFF_FFF8);
    put32(&mut fat, 4, 0x0FFF_FFFF);
    for n in &nodes {
        for k in 0..n.clusters {
            let c = n.first as u64 + k;
            let value = if k + 1 == n.clusters { 0x0FFF_FFFF } else { c as u32 + 1 };
            put32(&mut fat, c as usize * 4, value);
        }
    }

    let mut regions = vec![
        Region { sector: 0, data: boot.clone() },
        Region { sector: 1, data: info.clone() },
        Region { sector: 6, data: boot },
        Region { sector: 7, data: info },
        Region { sector: reserved, data: fat.clone() },
        Region { sector: reserved + fat_sectors, data: fat },
    ];
    for (i, n) in nodes.iter().enumerate() {
        if n.clusters == 0 {
            continue;
        }
        let mut data = match &n.kind {
            Kind::Dir(children) => {
                let mut d = Vec::new();
                if i != 0 {
                    d.extend_from_slice(&entry(b".          ", 0x10, 0, n.first, 0));
                    let up = if n.parent == 0 { 0 } else { nodes[n.parent].first };
                    d.extend_from_slice(&entry(b"..         ", 0x10, 0, up, 0));
                }
                for &c in children {
                    let ch = &nodes[c];
                    let (attr, size) = match ch.kind {
                        Kind::Dir(_) => (0x10, 0),
                        Kind::File(f) => (0x20, f.len() as u32),
                    };
                    d.extend_from_slice(&entry(&ch.short, attr, ch.case, ch.first, size));
                }
                d
            }
            Kind::File(f) => f.to_vec(),
        };
        data.resize((n.clusters * SECTOR) as usize, 0);
        regions.push(Region { sector: data_start + (n.first as u64 - 2), data });
    }
    regions.sort_by_key(|r| r.sector);
    Ok(regions)
}

/// The whole partition as one buffer (for image files and tests).
pub fn image(regions: &[Region], total_sectors: u64) -> Vec<u8> {
    let mut img = vec![0u8; (total_sectors * SECTOR) as usize];
    for r in regions {
        let at = (r.sector * SECTOR) as usize;
        img[at..at + r.data.len()].copy_from_slice(&r.data);
    }
    img
}
