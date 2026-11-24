use anathema_setup::fat32::{self, FileSpec};

const ESP_SECTORS: u64 = (200 << 20) / 512;

fn le16(b: &[u8], o: usize) -> u64 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap()) as u64
}
fn le32(b: &[u8], o: usize) -> u64 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as u64
}

/// A minimal FAT32 reader, independent of the writer: follows cluster chains from the root.
struct Fat<'a> {
    img: &'a [u8],
    fat: usize,
    data: usize,
    spc: usize,
    root: u64,
}

impl<'a> Fat<'a> {
    fn new(img: &'a [u8]) -> Self {
        assert_eq!(&img[510..512], &[0x55, 0xAA]);
        assert_eq!(&img[82..90], b"FAT32   ");
        let reserved = le16(img, 14) as usize;
        let nfats = img[16] as usize;
        let fat_sectors = le32(img, 36) as usize;
        Fat { img, fat: reserved * 512, data: (reserved + nfats * fat_sectors) * 512, spc: img[13] as usize, root: le32(img, 44) }
    }
    fn chain(&self, mut c: u64) -> Vec<u64> {
        let mut out = Vec::new();
        while (2..0x0FFF_FFF8).contains(&c) {
            out.push(c);
            c = le32(self.img, self.fat + 4 * c as usize) & 0x0FFF_FFFF;
            assert!(out.len() < 1_000_000, "chain loops");
        }
        out
    }
    fn read_chain(&self, first: u64) -> Vec<u8> {
        let mut out = Vec::new();
        for c in self.chain(first) {
            let at = self.data + (c as usize - 2) * self.spc * 512;
            out.extend_from_slice(&self.img[at..at + self.spc * 512]);
        }
        out
    }
    /// (8.3 name, attr, first cluster, size) of each live entry in a directory.
    fn dir(&self, first: u64) -> Vec<(String, u8, u64, u64)> {
        let raw = self.read_chain(first);
        let mut out = Vec::new();
        for e in raw.chunks(32) {
            if e[0] == 0 {
                break;
            }
            if e[0] == 0xE5 || e[11] == 0x0F {
                continue;
            }
            let name = String::from_utf8_lossy(&e[..8]).trim_end().to_string();
            let ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
            let full = if ext.is_empty() { name } else { format!("{name}.{ext}") };
            out.push((full, e[11], (le16(e, 20) << 16) | le16(e, 26), le32(e, 28)));
        }
        out
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let mut cluster = self.root;
        let parts: Vec<&str> = path.split('/').collect();
        for (i, p) in parts.iter().enumerate() {
            let e = self.dir(cluster).into_iter().find(|e| e.0 == p.to_ascii_uppercase())?;
            if i + 1 == parts.len() {
                let mut d = if e.2 == 0 { Vec::new() } else { self.read_chain(e.2) };
                d.truncate(e.3 as usize);
                return Some(d);
            }
            assert_eq!(e.1 & 0x10, 0x10, "{p} is a directory");
            cluster = e.2;
        }
        None
    }
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

#[test]
fn tree_with_four_files_round_trips() {
    let efi = pattern(300_000, 1);
    let kernel = pattern(21_128_776, 2);
    let xkc = pattern(40_000_000, 3);
    let xcp = pattern(498, 4);
    let files = [
        FileSpec { path: "EFI/BOOT/BOOTX64.EFI", data: &efi },
        FileSpec { path: "anathema/xkernel", data: &kernel },
        FileSpec { path: "anathema/xkc.bin", data: &xkc },
        FileSpec { path: "anathema/xcp.bin", data: &xcp },
    ];
    let regions = fat32::build(ESP_SECTORS, 2048, "ANATHEMA", &files).unwrap();
    let img = fat32::image(&regions, ESP_SECTORS);
    assert_eq!(img.len() as u64, ESP_SECTORS * 512);
    let fat = Fat::new(&img);
    for f in &files {
        assert!(fat.read(f.path).as_deref() == Some(f.data), "{} round trips", f.path);
    }
    // directories carry . and .. ; a subdirectory of the root points .. at cluster 0
    let anathema = fat.dir(fat.root).into_iter().find(|e| e.0 == "ANATHEMA").unwrap();
    let entries = fat.dir(anathema.2);
    assert_eq!((entries[0].0.as_str(), entries[0].2), (".", anathema.2));
    assert_eq!((entries[1].0.as_str(), entries[1].2), ("..", 0));
    // lower-case names keep their case for Windows through the NT case flags
    let raw = fat.read_chain(anathema.2);
    let xkernel = raw.chunks(32).find(|e| &e[..11] == b"XKERNEL    ").unwrap();
    assert_eq!(xkernel[12], 0x08);
    let xkc = raw.chunks(32).find(|e| &e[..11] == b"XKC     BIN").unwrap();
    assert_eq!(xkc[12], 0x18);

    #[cfg(target_os = "macos")]
    {
        let p = std::env::temp_dir().join(format!("anathema-esp-{}.img", std::process::id()));
        std::fs::write(&p, &img).unwrap();
        let out = std::process::Command::new("fsck_msdos").arg("-n").arg(&p).output().unwrap();
        let _ = std::fs::remove_file(&p);
        assert!(out.status.success(), "fsck_msdos: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn directory_spanning_several_clusters() {
    let names: Vec<String> = (0..40).map(|i| format!("d/f{i}.txt")).collect();
    let bodies: Vec<Vec<u8>> = (0..40).map(|i| pattern(100 + i, i as u8)).collect();
    let files: Vec<FileSpec> = names.iter().zip(&bodies).map(|(n, b)| FileSpec { path: n, data: b }).collect();
    let img = fat32::image(&fat32::build(ESP_SECTORS, 0, "T", &files).unwrap(), ESP_SECTORS);
    let fat = Fat::new(&img);
    for f in &files {
        assert!(fat.read(f.path).as_deref() == Some(f.data), "{}", f.path);
    }
}

#[test]
fn empty_file_has_no_clusters() {
    let img = fat32::image(&fat32::build(ESP_SECTORS, 0, "T", &[FileSpec { path: "anathema/xargs.txt", data: b"" }]).unwrap(), ESP_SECTORS);
    assert_eq!(Fat::new(&img).read("anathema/xargs.txt"), Some(vec![]));
}

#[test]
fn rejects_files_larger_than_partition() {
    let big = vec![0u8; 201 << 20];
    let err = fat32::build(ESP_SECTORS, 0, "T", &[FileSpec { path: "big.bin", data: &big }]).err().unwrap();
    assert!(err.contains("does not fit"), "{err}");
}

#[test]
fn rejects_non_8_3_name() {
    for bad in ["anathema/kernelcache.bin", "anathema/x.toolong", "a b.txt", "anathema/xkc.bin.map"] {
        let err = fat32::build(ESP_SECTORS, 0, "T", &[FileSpec { path: bad, data: b"1" }]).err().unwrap();
        assert!(err.contains("8.3"), "{bad}: {err}");
    }
}

#[test]
fn rejects_file_and_folder_with_one_name() {
    let err = fat32::build(ESP_SECTORS, 0, "T", &[FileSpec { path: "a", data: b"1" }, FileSpec { path: "a/b", data: b"2" }]).err().unwrap();
    assert!(err.contains("both"), "{err}");
}
