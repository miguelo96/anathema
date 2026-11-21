use super::nvme::{self, Drive, Nvme};
use crate::blog;
use core::ptr::read_unaligned;

pub const MAX_ENTRIES: usize = 24;

#[derive(Clone, Copy)]
pub struct Entry {
    pub index: u32,
    pub kind: [u8; 16],
    pub first: u64,
    pub last: u64,
    pub attrs: u64,
    pub name: [u8; 36],
}

pub struct Storage {
    pub state: u8,
    pub drive: Drive,
    pub valid: bool,
    pub header_crc: bool,
    pub entries_crc: bool,
    pub backup: u8,
    pub mbr: bool,
    pub disk: [u8; 16],
    pub first_usable: u64,
    pub last_usable: u64,
    pub declared: u32,
    pub used: u32,
    pub entries: [Entry; MAX_ENTRIES],
    pub shown: usize,
    pub note: &'static str,
}

const EMPTY_ENTRY: Entry = Entry { index: 0, kind: [0; 16], first: 0, last: 0, attrs: 0, name: [0; 36] };

impl Storage {
    pub const fn new() -> Storage {
        Storage {
            state: 0,
            drive: Drive { model: [0; 40], firmware: [0; 8], namespaces: 0, nsze: 0, lba_shift: 9, version: 0 },
            valid: false,
            header_crc: false,
            entries_crc: false,
            backup: 0,
            mbr: false,
            disk: [0; 16],
            first_usable: 0,
            last_usable: 0,
            declared: 0,
            used: 0,
            entries: [EMPTY_ENTRY; MAX_ENTRIES],
            shown: 0,
            note: "not run",
        }
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub fn guid_text(guid: &[u8; 16], out: &mut super::log::Line) {
    use core::fmt::Write;
    let a = u32::from_le_bytes([guid[0], guid[1], guid[2], guid[3]]);
    let b = u16::from_le_bytes([guid[4], guid[5]]);
    let c = u16::from_le_bytes([guid[6], guid[7]]);
    let _ = write!(out, "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-", a, b, c, guid[8], guid[9]);
    for byte in &guid[10..] {
        let _ = write!(out, "{:02x}", byte);
    }
}

pub fn kind_name(guid: &[u8; 16]) -> &'static str {
    let mut line = super::log::Line::new();
    guid_text(guid, &mut line);
    match line.as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" => "efi system",
        "e3c9e316-0b5c-4db8-817d-f92df00215ae" => "microsoft reserved",
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7" => "microsoft basic data",
        "de94bba4-06d1-4d40-a16a-bfd50179d6ac" => "windows recovery",
        "5808c8aa-7e8f-42e0-85d2-e1e90434cfb3" => "ldm metadata",
        "af9b60a0-1431-4f62-bc68-3311714a69ad" => "ldm data",
        "0fc63daf-8483-4772-8e79-3d69d8477de4" => "linux filesystem",
        "7c3457ef-0000-11aa-aa11-00306543ecac" => "apple apfs",
        "48465300-0000-11aa-aa11-00306543ecac" => "apple hfs+",
        "426f6f74-0000-11aa-aa11-00306543ecac" => "apple boot",
        "00000000-0000-0000-0000-000000000000" => "unused",
        _ => "other",
    }
}

pub fn read_disk(nvme: &mut Nvme, storage: &mut Storage) {
    let shift = storage.drive.lba_shift;
    let size = 1u64 << shift;
    let per_read = (8192u64 >> shift).max(1) as u32;
    if !(9..=12).contains(&shift) {
        storage.note = "unsupported sector size";
        storage.state = 2;
        return;
    }
    if nvme.read(0, 1).is_err() {
        storage.note = "read of sector 0 failed";
        storage.state = 2;
        return;
    }
    unsafe {
        let mbr = nvme.data as *const u8;
        let sig = u16::from_le_bytes([*mbr.add(510), *mbr.add(511)]);
        let kind = *mbr.add(450);
        storage.mbr = sig == 0xaa55;
        blog!("gpt: sector 0 signature {:#06x}, first partition type {:#04x}", sig, kind);
    }
    if nvme.read(1, 1).is_err() {
        storage.note = "read of sector 1 failed";
        storage.state = 2;
        return;
    }
    let mut header = [0u8; 92];
    unsafe {
        core::ptr::copy_nonoverlapping(nvme.data as *const u8, header.as_mut_ptr(), 92);
    }
    if &header[..8] != b"EFI PART" {
        blog!("gpt: no gpt header at sector 1");
        storage.note = "no gpt header";
        storage.state = 1;
        return;
    }
    let hsize = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    let stored = u32::from_le_bytes(header[16..20].try_into().unwrap());
    let current = u64::from_le_bytes(header[24..32].try_into().unwrap());
    let backup_lba = u64::from_le_bytes(header[32..40].try_into().unwrap());
    storage.first_usable = u64::from_le_bytes(header[40..48].try_into().unwrap());
    storage.last_usable = u64::from_le_bytes(header[48..56].try_into().unwrap());
    storage.disk.copy_from_slice(&header[56..72]);
    let entries_lba = u64::from_le_bytes(header[72..80].try_into().unwrap());
    let count = u32::from_le_bytes(header[80..84].try_into().unwrap());
    let esize = u32::from_le_bytes(header[84..88].try_into().unwrap()) as usize;
    let entries_crc = u32::from_le_bytes(header[88..92].try_into().unwrap());
    storage.declared = count;
    storage.valid = true;
    let mut check = header;
    check[16..20].copy_from_slice(&[0; 4]);
    storage.header_crc = hsize == 92 && crc32(&check[..hsize.min(92)]) == stored;
    blog!(
        "gpt: header at lba {}, size {}, crc {}, revision {:#x}, backup at lba {}, usable {}..{}, entries at lba {} ({} x {} bytes)",
        current,
        hsize,
        if storage.header_crc { "ok" } else { "BAD" },
        u32::from_le_bytes(header[8..12].try_into().unwrap()),
        backup_lba,
        storage.first_usable,
        storage.last_usable,
        entries_lba,
        count,
        esize
    );
    if esize < 128 || esize > 512 || count == 0 || count > 1024 {
        storage.note = "gpt entry table geometry unreasonable";
        storage.state = 1;
        return;
    }
    let total = count as usize * esize;
    let mut crc_state = 0xffff_ffffu32;
    let mut index = 0u32;
    let mut done = 0usize;
    let mut sector = entries_lba;
    let mut carry = [0u8; 512];
    let mut carry_len = 0usize;
    while done < total {
        let want = ((total - done) as u64).div_ceil(size).min(per_read as u64) as u32;
        if nvme.read(sector, want).is_err() {
            storage.note = "read of the entry table failed";
            storage.state = 2;
            return;
        }
        let have = ((want as u64) * size).min((total - done) as u64) as usize;
        let chunk = unsafe { core::slice::from_raw_parts(nvme.data as *const u8, have) };
        for &byte in chunk {
            crc_state ^= byte as u32;
            for _ in 0..8 {
                crc_state = if crc_state & 1 != 0 { (crc_state >> 1) ^ 0xedb8_8320 } else { crc_state >> 1 };
            }
        }
        let mut offset = 0usize;
        while offset < have {
            let take = (esize - carry_len).min(have - offset);
            if carry_len + take > carry.len() {
                break;
            }
            carry[carry_len..carry_len + take].copy_from_slice(&chunk[offset..offset + take]);
            carry_len += take;
            offset += take;
            if carry_len == esize {
                let kind: [u8; 16] = carry[..16].try_into().unwrap();
                if kind != [0; 16] {
                    storage.used += 1;
                    if storage.shown < MAX_ENTRIES {
                        let mut entry = EMPTY_ENTRY;
                        entry.index = index + 1;
                        entry.kind = kind;
                        entry.first = u64::from_le_bytes(carry[32..40].try_into().unwrap());
                        entry.last = u64::from_le_bytes(carry[40..48].try_into().unwrap());
                        entry.attrs = u64::from_le_bytes(carry[48..56].try_into().unwrap());
                        let mut at = 0;
                        for unit in 0..36 {
                            if 56 + unit * 2 + 1 >= esize {
                                break;
                            }
                            let code = u16::from_le_bytes([carry[56 + unit * 2], carry[57 + unit * 2]]);
                            if code == 0 {
                                break;
                            }
                            entry.name[at] = if code < 128 && code >= 32 { code as u8 } else { b'?' };
                            at += 1;
                        }
                        storage.entries[storage.shown] = entry;
                        storage.shown += 1;
                    }
                }
                index += 1;
                carry_len = 0;
            }
        }
        done += have;
        sector += want as u64;
    }
    storage.entries_crc = !crc_state == entries_crc;
    blog!("gpt: entry table crc {}, {} of {} entries in use", if storage.entries_crc { "ok" } else { "BAD" }, storage.used, count);
    for i in 0..storage.shown {
        let entry = storage.entries[i];
        let mut guid = super::log::Line::new();
        guid_text(&entry.kind, &mut guid);
        blog!(
            "gpt: #{} {} [{}] lba {}..{} ({} MiB) attrs {:#x} name \"{}\"",
            entry.index,
            kind_name(&entry.kind),
            guid.as_str(),
            entry.first,
            entry.last,
            ((entry.last + 1).saturating_sub(entry.first) << shift) >> 20,
            entry.attrs,
            nvme::trim(&entry.name)
        );
    }
    let mut disk = super::log::Line::new();
    guid_text(&storage.disk, &mut disk);
    blog!("gpt: disk guid {}", disk.as_str());
    storage.backup = 0;
    if backup_lba != 0 && backup_lba < storage.drive.nsze {
        if nvme.read(backup_lba, 1).is_ok() {
            let signature = unsafe { core::slice::from_raw_parts(nvme.data as *const u8, 8) };
            let same_disk = unsafe { read_unaligned((nvme.data + 56) as *const [u8; 16]) } == storage.disk;
            storage.backup = if signature == b"EFI PART" && same_disk { 1 } else { 2 };
        }
    }
    blog!(
        "gpt: backup header at lba {}: {}",
        backup_lba,
        match storage.backup {
            1 => "present and matches",
            2 => "missing or different",
            _ => "not read",
        }
    );
    storage.state = 1;
    storage.note = "read only, nothing written";
}
