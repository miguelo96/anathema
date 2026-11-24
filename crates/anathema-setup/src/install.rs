use crate::disk::{self, Disk, NewPart, Part};
use anathema_setup::{fat32, kernel_files};
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    UpdateBoot,
    Fresh,
}

pub struct Context<'a> {
    pub dry_run: bool,
    pub log: &'a mut dyn FnMut(String),
    pub progress: &'a mut dyn FnMut(f32, &str),
}

pub const ESP_BYTES: u64 = 200 << 20;
const MIB: u64 = 1 << 20;

/// Full path to a program in the Windows system directory (normally `C:\Windows\System32`).
///
/// This process runs elevated, so system tools must never be started by bare name: Windows
/// looks in the folder of the running exe first (usually Downloads), so a stray `format.com`
/// there would run as administrator.
fn system_tool(name: &str) -> std::io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).join(name))
}

pub fn data_volume(disk: &Disk) -> Option<&Part> {
    disk.parts
        .iter()
        .filter(|p| !p.is_esp() && p.letter.is_some())
        .find(|p| {
            let root = format!("{}:\\", p.letter.unwrap());
            Path::new(&root).join("stela").is_dir() || Path::new(&root).join("anathema").is_dir()
        })
}

pub fn esp_part(disk: &Disk) -> Option<&Part> {
    disk.parts.iter().find(|p| p.is_esp()).or_else(|| {
        disk.parts.iter().find(|p| {
            p.letter
                .is_some_and(|l| Path::new(&format!("{l}:\\EFI")).is_dir())
        })
    })
}

pub fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => directory_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn prepared_bytes(disk: &Disk) -> Option<u64> {
    let part = data_volume(disk)?;
    let root = PathBuf::from(format!("{}:\\", part.letter?));
    let stela = directory_bytes(&root.join("stela"));
    let anathema = directory_bytes(&root.join("anathema"));
    Some(stela + anathema)
}

pub fn backup_target(disk: &Disk) -> Option<PathBuf> {
    let part = data_volume(disk)
        .or_else(|| disk.parts.iter().find(|p| !p.is_esp() && p.letter.is_some()))
        .or_else(|| esp_part(disk))?;
    Some(PathBuf::from(format!("{}:\\anathema-backup-efi", part.letter?)))
}

fn copy_tree(from: &Path, to: &Path, count: &mut u64, total: u64, ctx: &mut Context) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &target, count, total, ctx)?;
        } else {
            fs::copy(entry.path(), &target)
                .map_err(|e| format!("cannot copy {}: {e}", entry.path().display()))?;
            *count += 1;
            (ctx.progress)(0.1 + 0.4 * (*count as f32 / total.max(1) as f32).min(1.0), "Backing up the EFI folder");
        }
    }
    Ok(())
}

fn count_files(path: &Path) -> u64 {
    fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => count_files(&e.path()),
                    Ok(_) => 1,
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn esp_letter(disk: &Disk, part: &Part, ctx: &mut Context) -> Result<char, String> {
    if let Some(letter) = part.letter {
        return Ok(letter);
    }
    if ctx.dry_run {
        (ctx.log)("The EFI partition has no drive letter, one would be assigned".into());
        return Ok('?');
    }
    let volume = disk::volume_for_partition(disk.number, part.offset)
        .ok_or("Windows has no volume for the EFI partition")?;
    let letter = disk::assign_letter(&volume)?;
    (ctx.log)(format!("Assigned {letter}: to the EFI partition"));
    Ok(letter)
}

pub fn update_boot(disk: &Disk, efi: &[u8], files: &[fat32::FileSpec], ctx: &mut Context) -> Result<(), String> {
    check_target(disk)?;
    let part = esp_part(disk).ok_or("this stick has no EFI partition to update")?.clone();
    (ctx.progress)(0.05, "Locating the EFI partition");
    let letter = esp_letter(disk, &part, ctx)?;
    let esp = PathBuf::from(format!("{letter}:\\"));
    let efi_dir = esp.join("EFI");
    let backup = backup_target(disk).ok_or("no volume to hold the backup")?;
    if efi_dir.is_dir() {
        let total = count_files(&efi_dir);
        let size = directory_bytes(&efi_dir);
        (ctx.log)(format!("Backing up {} ({total} files, {} KB) to {}", efi_dir.display(), size / 1024, backup.display()));
        let backup_root = backup.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let free = disk::free_bytes(&backup_root.display().to_string());
        if !ctx.dry_run && free < size + (16 << 20) {
            return Err(format!("not enough free space for the backup on {}", backup_root.display()));
        }
        if !ctx.dry_run {
            if backup.exists() {
                let moved = (1..)
                    .map(|n| backup.with_file_name(format!("anathema-backup-efi-{n}")))
                    .find(|p| !p.exists())
                    .ok_or("no free backup name")?;
                fs::rename(&backup, &moved).map_err(|e| format!("cannot move the old backup: {e}"))?;
                (ctx.log)(format!("Older backup kept as {}", moved.display()));
            }
            let mut count = 0;
            copy_tree(&efi_dir, &backup, &mut count, total, ctx)?;
            (ctx.log)(format!("Backup complete: {count} files"));
        }
    } else {
        (ctx.log)("The EFI partition has no EFI folder, nothing to back up".into());
    }

    // everything that is written, with what it replaces, must fit before anything is written
    let esp_path = |p: &str| esp.join(p.replace('/', "\\"));
    let mut planned: Vec<fat32::FileSpec> = files.to_vec();
    planned.push(fat32::FileSpec { path: "EFI/anathema/anathema.efi", data: efi });
    let existing: Vec<(String, u64)> = planned
        .iter()
        .filter_map(|f| fs::metadata(esp_path(f.path)).ok().map(|m| (f.path.to_string(), m.len())))
        .collect();
    let esp_free = disk::free_bytes(&format!("{letter}:\\"));
    if !ctx.dry_run {
        kernel_files::update_plan(esp_free, &existing, &planned)?;
    }

    (ctx.progress)(0.55, "Writing the Anathema boot image");
    let boot_dir = efi_dir.join("BOOT");
    let anathema_dir = efi_dir.join("anathema");
    let target = boot_dir.join("BOOTX64.EFI");
    (ctx.log)(format!("Writing {} ({} bytes)", target.display(), efi.len()));
    if !ctx.dry_run {
        fs::create_dir_all(&boot_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&anathema_dir).map_err(|e| e.to_string())?;
        let staged = boot_dir.join("BOOTX64.NEW");
        fs::write(&staged, efi).map_err(|e| format!("cannot write the boot image: {e}"))?;
        fs::rename(&staged, &target).map_err(|e| format!("cannot replace BOOTX64.EFI: {e}"))?;
        fs::write(anathema_dir.join("anathema.efi"), efi).map_err(|e| e.to_string())?;
        (ctx.progress)(0.85, "Verifying");
        let written = fs::read(&target).map_err(|e| e.to_string())?;
        if written != efi {
            return Err("verification failed: BOOTX64.EFI differs from the image".into());
        }
        (ctx.log)("Verified BOOTX64.EFI".into());
    }
    let extra: Vec<&fat32::FileSpec> = files.iter().filter(|f| f.path != "EFI/BOOT/BOOTX64.EFI").collect();
    for (i, f) in extra.iter().enumerate() {
        let target = esp_path(f.path);
        (ctx.log)(format!("Writing {} ({} bytes)", target.display(), f.data.len()));
        (ctx.progress)(0.85 + 0.1 * i as f32 / extra.len().max(1) as f32, "Writing the kernel files");
        if ctx.dry_run {
            continue;
        }
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let staged = target.with_file_name(format!("{name}.new"));
        fs::write(&staged, f.data).map_err(|e| format!("cannot write {}: {e}", staged.display()))?;
        fs::rename(&staged, &target).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
        if fs::read(&target).map_err(|e| e.to_string())? != f.data {
            return Err(format!("verification failed: {} differs from what was written", target.display()));
        }
    }
    if !ctx.dry_run {
        disk::flush_letter(letter);
        (ctx.log)("Verified the kernel files and flushed the volume".into());
    }
    (ctx.progress)(1.0, "Finished");
    Ok(())
}

fn check_target(disk: &Disk) -> Result<(), String> {
    let fresh = disk::refresh_one(disk.number)?;
    if disk.number == 0 || !fresh.selectable || fresh.bytes != disk.bytes {
        return Err(format!("disk {} is not an allowed target", disk.number));
    }
    Ok(())
}

pub fn fresh_install(disk: &Disk, files: &[fat32::FileSpec], ctx: &mut Context) -> Result<(), String> {
    check_target(disk)?;
    if disk.sector != 512 {
        return Err(format!("sector size {} is not supported, only 512", disk.sector));
    }
    let esp_offset = MIB;
    let data_offset = esp_offset + ESP_BYTES + MIB;
    let data_length = (disk.bytes.saturating_sub(MIB) / MIB) * MIB - data_offset;
    if disk.bytes < data_offset + 512 * MIB {
        return Err("stick is too small".into());
    }
    (ctx.log)(format!(
        "Layout: GPT, EFI {} MiB at 1 MiB, data {} MB after it",
        ESP_BYTES >> 20,
        data_length / 1_000_000
    ));
    (ctx.progress)(0.05, "Unmounting the stick");
    for part in &disk.parts {
        if let Some(letter) = part.letter {
            (ctx.log)(format!("Unmounting {letter}:"));
            if !ctx.dry_run {
                disk::lock_and_dismount(letter)?;
            }
        }
    }
    (ctx.progress)(0.15, "Writing the partition table");
    (ctx.log)("Writing a new GPT (the old partitions are erased)".into());
    let parts = [
        NewPart { offset: esp_offset, length: ESP_BYTES, kind: disk::ESP_GUID, name: "EFI System Partition" },
        NewPart { offset: data_offset, length: data_length, kind: disk::BASIC_GUID, name: "ANATHEMA" },
    ];
    if !ctx.dry_run {
        disk::write_gpt_layout(disk.number, disk.bytes, &parts)?;
        thread::sleep(Duration::from_millis(1500));
    }
    (ctx.progress)(0.35, "Formatting the EFI partition");
    let regions = fat32::build(ESP_BYTES / 512, (esp_offset / 512) as u32, "ANATHEMA", files)?;
    for f in files {
        (ctx.log)(format!("FAT32 EFI partition: {} ({} bytes)", f.path, f.data.len()));
    }
    if !ctx.dry_run {
        let mut guard = None;
        for _ in 0..8 {
            if let Some(volume) = disk::volume_for_partition(disk.number, esp_offset) {
                guard = disk::lock_volume(&volume).ok();
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(3)
            .open(format!(r"\\.\PhysicalDrive{}", disk.number))
            .map_err(|e| format!("cannot open the disk for writing: {e}"))?;
        let count = regions.len().max(1);
        for (i, region) in regions.iter().enumerate() {
            file.seek(SeekFrom::Start(esp_offset + region.sector * fat32::SECTOR))
                .map_err(|e| e.to_string())?;
            file.write_all(&region.data).map_err(|e| format!("write failed: {e}"))?;
            (ctx.progress)(0.35 + 0.35 * (i + 1) as f32 / count as f32, "Formatting the EFI partition");
        }
        file.sync_all().map_err(|e| e.to_string())?;
        let mut readback = vec![0u8; 512];
        let mut check = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(format!(r"\\.\PhysicalDrive{}", disk.number))
            .map_err(|e| e.to_string())?;
        check.seek(SeekFrom::Start(esp_offset)).map_err(|e| e.to_string())?;
        std::io::Read::read_exact(&mut check, &mut readback).map_err(|e| e.to_string())?;
        if readback[510] != 0x55 || readback[511] != 0xAA || &readback[82..90] != b"FAT32   " {
            return Err("verification failed: the EFI boot sector did not read back".into());
        }
        drop(guard);
        (ctx.log)("EFI partition written and verified".into());
    }
    (ctx.progress)(0.75, "Preparing the data partition");
    if ctx.dry_run {
        (ctx.log)("Data partition would be formatted exFAT, label ANATHEMA".into());
    } else {
        let mut formatted = false;
        for _ in 0..12 {
            let existing = disk::refresh_one(disk.number)
                .ok()
                .and_then(|d| d.parts.iter().find(|p| p.offset == data_offset).and_then(|p| p.letter));
            let letter = existing.or_else(|| {
                disk::volume_for_partition(disk.number, data_offset)
                    .and_then(|volume| disk::assign_letter(&volume).ok())
            });
            if let Some(letter) = letter {
                {
                    let out = system_tool("format.com").and_then(|format| {
                        Command::new(format)
                            .args([&format!("{letter}:"), "/FS:exFAT", "/V:ANATHEMA", "/Q", "/Y"])
                            .output()
                    });
                    match out {
                        Ok(o) if o.status.success() => {
                            (ctx.log)(format!("Data partition formatted exFAT as {letter}:"));
                            formatted = true;
                        }
                        Ok(o) => (ctx.log)(format!("format.com failed: {}", String::from_utf8_lossy(&o.stdout).trim())),
                        Err(e) => (ctx.log)(format!("format.com not available: {e}")),
                    }
                }
                break;
            }
            thread::sleep(Duration::from_millis(700));
        }
        if !formatted {
            (ctx.log)("Data partition left unformatted, macOS media prep will format it later".into());
        }
    }
    (ctx.progress)(1.0, "Finished");
    Ok(())
}
