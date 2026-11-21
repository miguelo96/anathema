pub const SECTOR: u64 = 512;

pub struct Region {
    pub sector: u64,
    pub data: Vec<u8>,
}

fn put16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    let date: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
    put16(&mut e, 16, date);
    put16(&mut e, 24, date);
    put16(&mut e, 18, date);
    put16(&mut e, 20, (cluster >> 16) as u16);
    put16(&mut e, 26, cluster as u16);
    put32(&mut e, 28, size);
    e
}

pub fn build(total_sectors: u64, hidden: u32, label: &str, loader: &[u8]) -> Result<Vec<Region>, String> {
    if total_sectors < 66_000 {
        return Err("partition too small for FAT32".into());
    }
    let reserved: u64 = 32;
    let fat_sectors = (total_sectors - reserved + 2).div_ceil(130);
    let data_start = reserved + 2 * fat_sectors;
    let clusters = total_sectors - data_start;
    if clusters < 65_525 {
        return Err("partition has too few clusters for FAT32".into());
    }
    let file_clusters = (loader.len() as u64).div_ceil(SECTOR).max(1);
    if file_clusters + 5 > clusters {
        return Err("boot image does not fit".into());
    }

    let mut boot = vec![0u8; 512];
    boot[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    boot[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut boot, 11, 512);
    boot[13] = 1;
    put16(&mut boot, 14, reserved as u16);
    boot[16] = 2;
    boot[21] = 0xF8;
    put16(&mut boot, 24, 63);
    put16(&mut boot, 26, 255);
    put32(&mut boot, 28, hidden);
    put32(&mut boot, 32, total_sectors as u32);
    put32(&mut boot, 36, fat_sectors as u32);
    put32(&mut boot, 44, 2);
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
    put32(&mut info, 488, 0xFFFF_FFFF);
    put32(&mut info, 492, 0xFFFF_FFFF);
    info[510] = 0x55;
    info[511] = 0xAA;

    let mut fat = vec![0u8; (fat_sectors * SECTOR) as usize];
    let mut set = |index: usize, value: u32| put32(&mut fat, index * 4, value);
    set(0, 0x0FFF_FFF8);
    set(1, 0x0FFF_FFFF);
    set(2, 0x0FFF_FFFF);
    set(3, 0x0FFF_FFFF);
    set(4, 0x0FFF_FFFF);
    for i in 0..file_clusters as usize {
        let cluster = 5 + i;
        let next = if i as u64 + 1 == file_clusters { 0x0FFF_FFFF } else { cluster as u32 + 1 };
        set(cluster, next);
    }

    let dot = *b".          ";
    let dotdot = *b"..         ";
    let mut root = vec![0u8; 512];
    root[..32].copy_from_slice(&entry(b"EFI        ", 0x10, 3, 0));
    let mut efi_dir = vec![0u8; 512];
    efi_dir[..32].copy_from_slice(&entry(&dot, 0x10, 3, 0));
    efi_dir[32..64].copy_from_slice(&entry(&dotdot, 0x10, 0, 0));
    efi_dir[64..96].copy_from_slice(&entry(b"BOOT       ", 0x10, 4, 0));
    let mut boot_dir = vec![0u8; 512];
    boot_dir[..32].copy_from_slice(&entry(&dot, 0x10, 4, 0));
    boot_dir[32..64].copy_from_slice(&entry(&dotdot, 0x10, 3, 0));
    boot_dir[64..96].copy_from_slice(&entry(b"BOOTX64 EFI", 0x20, 5, loader.len() as u32));

    let mut data = loader.to_vec();
    data.resize((file_clusters * SECTOR) as usize, 0);

    let mut regions = vec![
        Region { sector: 0, data: boot.clone() },
        Region { sector: 1, data: info.clone() },
        Region { sector: 6, data: boot },
        Region { sector: 7, data: info },
        Region { sector: reserved, data: fat.clone() },
        Region { sector: reserved + fat_sectors, data: fat },
        Region { sector: data_start, data: root },
        Region { sector: data_start + 1, data: efi_dir },
        Region { sector: data_start + 2, data: boot_dir },
        Region { sector: data_start + 3, data },
    ];
    regions.sort_by_key(|r| r.sector);
    Ok(regions)
}
