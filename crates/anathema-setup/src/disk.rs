use std::{
    ffi::{c_void, OsStr},
    mem,
    os::windows::ffi::OsStrExt,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceExW,
        GetLogicalDrives, GetVolumeInformationW, SetVolumeMountPointW, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        Ioctl::*,
        IO::DeviceIoControl,
    },
};

const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

pub static ALLOW_VIRTUAL: AtomicBool = AtomicBool::new(false);

pub const ESP_GUID: [u8; 16] = guid(0xC12A7328, 0xF81F, 0x11D2, [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B]);
pub const BASIC_GUID: [u8; 16] = guid(0xEBD0A0A2, 0xB9E5, 0x4433, [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]);

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7],
    ]
}

pub fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}

pub struct Handle(pub HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn open(path: &str, access: u32) -> Result<Handle, String> {
    let handle = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(format!("cannot open {path} (error {})", unsafe { GetLastError() }))
    } else {
        Ok(Handle(handle))
    }
}

pub fn ioctl(
    handle: &Handle,
    code: u32,
    input: *const c_void,
    input_len: usize,
    output: *mut c_void,
    output_len: usize,
) -> Result<u32, u32> {
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            code,
            input,
            input_len as u32,
            output,
            output_len as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(returned)
    }
}

#[derive(Clone, Debug)]
pub struct Part {
    pub number: u32,
    pub offset: u64,
    pub length: u64,
    pub kind: [u8; 16],
    pub letter: Option<char>,
    pub label: String,
    pub fs: String,
}

impl Part {
    pub fn is_esp(&self) -> bool {
        self.kind == ESP_GUID
    }
}

#[derive(Clone, Debug)]
pub struct Disk {
    pub number: u32,
    pub model: String,
    pub bytes: u64,
    pub bus: u32,
    pub removable: bool,
    pub system: bool,
    pub parts: Vec<Part>,
    pub selectable: bool,
    pub reason: String,
    pub sector: u32,
}

fn bus_name(bus: u32) -> &'static str {
    match bus {
        7 => "USB",
        12 => "SD",
        13 => "MMC",
        17 => "NVMe",
        11 => "SATA",
        8 => "RAID",
        10 => "SAS",
        15 => "virtual",
        _ => "other",
    }
}

fn string_at(buf: &[u8], offset: u32) -> String {
    if offset == 0 || offset as usize >= buf.len() {
        return String::new();
    }
    let tail = &buf[offset as usize..];
    let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).trim().to_owned()
}

fn letters() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i as u8) as char)
        .collect()
}

fn letter_location(letter: char) -> Option<(u32, u32)> {
    let handle = open(&format!(r"\\.\{letter}:"), 0).ok()?;
    let mut number: STORAGE_DEVICE_NUMBER = unsafe { mem::zeroed() };
    ioctl(
        &handle,
        IOCTL_STORAGE_GET_DEVICE_NUMBER,
        ptr::null(),
        0,
        &mut number as *mut _ as *mut c_void,
        mem::size_of::<STORAGE_DEVICE_NUMBER>(),
    )
    .ok()?;
    Some((number.DeviceNumber, number.PartitionNumber))
}

pub fn volume_info(letter: char) -> (String, String) {
    let root = wide(&format!(r"{letter}:\"));
    let mut label = [0u16; 64];
    let mut fs = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return (String::new(), String::new());
    }
    let text = |buf: &[u16]| {
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    };
    (text(&label), text(&fs))
}

pub fn free_bytes(path: &str) -> u64 {
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(wide(path).as_ptr(), &mut free, ptr::null_mut(), ptr::null_mut())
    };
    if ok == 0 {
        0
    } else {
        free
    }
}

fn read_layout(handle: &Handle) -> Vec<(u32, u64, u64, [u8; 16], bool)> {
    let mut buf = vec![0u8; 1 << 16];
    let Ok(_) = ioctl(
        handle,
        IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
        ptr::null(),
        0,
        buf.as_mut_ptr() as *mut c_void,
        buf.len(),
    ) else {
        return Vec::new();
    };
    let head: DRIVE_LAYOUT_INFORMATION_EX = unsafe { ptr::read_unaligned(buf.as_ptr() as *const _) };
    let base = mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
    let step = mem::size_of::<PARTITION_INFORMATION_EX>();
    let gpt = head.PartitionStyle == PARTITION_STYLE_GPT as u32;
    let mut out = Vec::new();
    for i in 0..head.PartitionCount as usize {
        let at = base + i * step;
        if at + step > buf.len() {
            break;
        }
        let entry: PARTITION_INFORMATION_EX =
            unsafe { ptr::read_unaligned(buf.as_ptr().add(at) as *const _) };
        if entry.PartitionLength <= 0 || entry.PartitionNumber == 0 {
            continue;
        }
        let mut kind = [0u8; 16];
        if gpt {
            let g = unsafe { entry.Anonymous.Gpt.PartitionType };
            kind[..4].copy_from_slice(&g.data1.to_le_bytes());
            kind[4..6].copy_from_slice(&g.data2.to_le_bytes());
            kind[6..8].copy_from_slice(&g.data3.to_le_bytes());
            kind[8..].copy_from_slice(&g.data4);
        }
        out.push((
            entry.PartitionNumber,
            entry.StartingOffset as u64,
            entry.PartitionLength as u64,
            kind,
            gpt,
        ));
    }
    out
}

pub fn list() -> Result<Vec<Disk>, String> {
    let system_letter = std::env::var("SystemDrive")
        .ok()
        .and_then(|s| s.chars().next())
        .unwrap_or('C')
        .to_ascii_uppercase();
    let located: Vec<(char, u32, u32)> = letters()
        .into_iter()
        .filter_map(|l| letter_location(l).map(|(d, p)| (l, d, p)))
        .collect();
    let system_disk = located.iter().find(|(l, _, _)| *l == system_letter).map(|(_, d, _)| *d);
    let mut disks = Vec::new();
    for number in 0..32u32 {
        let Ok(handle) = open(&format!(r"\\.\PhysicalDrive{number}"), 0) else {
            continue;
        };
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut buf = vec![0u8; 2048];
        let described = ioctl(
            &handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const c_void,
            mem::size_of::<STORAGE_PROPERTY_QUERY>(),
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
        )
        .is_ok();
        let (bus, removable, model) = if described {
            let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { ptr::read_unaligned(buf.as_ptr() as *const _) };
            let vendor = string_at(&buf, d.VendorIdOffset);
            let product = string_at(&buf, d.ProductIdOffset);
            (
                d.BusType as u32,
                d.RemovableMedia,
                format!("{vendor} {product}").trim().to_owned(),
            )
        } else {
            (0, false, String::new())
        };
        let mut length: GET_LENGTH_INFORMATION = unsafe { mem::zeroed() };
        let bytes = match open(&format!(r"\\.\PhysicalDrive{number}"), GENERIC_READ).and_then(|h| ioctl(
            &h,
            IOCTL_DISK_GET_LENGTH_INFO,
            ptr::null(),
            0,
            &mut length as *mut _ as *mut c_void,
            mem::size_of::<GET_LENGTH_INFORMATION>(),
        ).map_err(|e| e.to_string())) {
            Ok(_) => length.Length as u64,
            Err(_) => 0,
        };
        let mut geometry: DISK_GEOMETRY_EX = unsafe { mem::zeroed() };
        let sector = match ioctl(
            &handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            ptr::null(),
            0,
            &mut geometry as *mut _ as *mut c_void,
            mem::size_of::<DISK_GEOMETRY_EX>(),
        ) {
            Ok(_) => geometry.Geometry.BytesPerSector,
            Err(_) => 512,
        };
        let bytes = if bytes == 0 { geometry.DiskSize as u64 } else { bytes };
        let parts = read_layout(&handle)
            .into_iter()
            .map(|(pn, offset, length, kind, _)| {
                let letter = located
                    .iter()
                    .find(|(_, d, p)| *d == number && *p == pn)
                    .map(|(l, _, _)| *l);
                let (label, fs) = letter.map(volume_info).unwrap_or_default();
                Part { number: pn, offset, length, kind, letter, label, fs }
            })
            .collect();
        let system = number == 0 || system_disk == Some(number);
        let (selectable, reason) = judge(number, system, bus, removable, bytes);
        disks.push(Disk {
            number,
            model: if model.is_empty() { "Unknown disk".into() } else { model },
            bytes,
            bus,
            removable,
            system,
            parts,
            selectable,
            reason,
            sector,
        });
    }
    Ok(disks)
}

fn judge(number: u32, system: bool, bus: u32, removable: bool, bytes: u64) -> (bool, String) {
    if number == 0 {
        return (false, "disk 0 is never allowed".into());
    }
    if system {
        return (false, "this disk runs Windows".into());
    }
    let virt = bus == 15 && ALLOW_VIRTUAL.load(Ordering::Relaxed);
    let usb_like = matches!(bus, 7 | 12 | 13);
    if !usb_like && !virt {
        return (false, format!("{} disk, not a removable USB stick", bus_name(bus)));
    }
    if !virt && !removable && bytes > 256_000_000_000 {
        return (false, "fixed USB drive larger than a stick".into());
    }
    if bytes < 1_000_000_000 {
        return (false, "smaller than 1 GB".into());
    }
    (true, format!("removable {}", bus_name(bus)))
}

pub fn refresh_one(number: u32) -> Result<Disk, String> {
    list()?
        .into_iter()
        .find(|d| d.number == number)
        .ok_or_else(|| format!("disk {number} is gone"))
}

pub fn free_letter() -> Option<char> {
    let used = unsafe { GetLogicalDrives() };
    ('D'..='Z').rev().find(|c| used & (1 << (*c as u8 - b'A')) == 0)
}

pub fn volume_for_partition(disk: u32, offset: u64) -> Option<String> {
    let mut name = [0u16; 260];
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut found = None;
    loop {
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        let path = String::from_utf16_lossy(&name[..end]);
        let bare = path.trim_end_matches('\\');
        if let Ok(handle) = open(bare, 0) {
            let mut buf = vec![0u8; 1024];
            if ioctl(
                &handle,
                IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                ptr::null(),
                0,
                buf.as_mut_ptr() as *mut c_void,
                buf.len(),
            )
            .is_ok()
            {
                let head: VOLUME_DISK_EXTENTS = unsafe { ptr::read_unaligned(buf.as_ptr() as *const _) };
                if head.NumberOfDiskExtents >= 1
                    && head.Extents[0].DiskNumber == disk
                    && head.Extents[0].StartingOffset as u64 == offset
                {
                    found = Some(path.clone());
                }
            }
        }
        if found.is_some() {
            break;
        }
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    unsafe {
        FindVolumeClose(find);
    }
    found
}

pub fn assign_letter(volume: &str) -> Result<char, String> {
    let letter = free_letter().ok_or("no free drive letter")?;
    let ok = unsafe { SetVolumeMountPointW(wide(&format!(r"{letter}:\")).as_ptr(), wide(volume).as_ptr()) };
    if ok == 0 {
        Err(format!("cannot assign {letter}: (error {})", unsafe { GetLastError() }))
    } else {
        Ok(letter)
    }
}

pub fn lock_and_dismount(letter: char) -> Result<(), String> {
    let handle = open(&format!(r"\\.\{letter}:"), GENERIC_READ | GENERIC_WRITE)?;
    for _ in 0..8 {
        if ioctl(&handle, FSCTL_LOCK_VOLUME, ptr::null(), 0, ptr::null_mut(), 0).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    ioctl(&handle, FSCTL_DISMOUNT_VOLUME, ptr::null(), 0, ptr::null_mut(), 0)
        .map_err(|e| format!("cannot dismount {letter}: (error {e})"))?;
    Ok(())
}

pub fn flush_letter(letter: char) {
    if let Ok(handle) = open(&format!(r"\\.\{letter}:"), GENERIC_READ | GENERIC_WRITE) {
        unsafe {
            windows_sys::Win32::Storage::FileSystem::FlushFileBuffers(handle.0);
        }
    }
}

pub fn random_guid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut out = [0u8; 16];
    for chunk in out.chunks_mut(8) {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    out[7] = (out[7] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

fn to_guid(bytes: [u8; 16]) -> windows_sys::core::GUID {
    windows_sys::core::GUID {
        data1: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        data2: u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
        data3: u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
        data4: bytes[8..16].try_into().unwrap(),
    }
}

pub struct NewPart {
    pub offset: u64,
    pub length: u64,
    pub kind: [u8; 16],
    pub name: &'static str,
}

pub fn write_gpt_layout(number: u32, bytes: u64, parts: &[NewPart]) -> Result<(), String> {
    let handle = open(&format!(r"\\.\PhysicalDrive{number}"), GENERIC_READ | GENERIC_WRITE)?;
    let _ = ioctl(&handle, IOCTL_DISK_DELETE_DRIVE_LAYOUT, ptr::null(), 0, ptr::null_mut(), 0);
    let disk_id = random_guid();
    let mut create: CREATE_DISK = unsafe { mem::zeroed() };
    create.PartitionStyle = PARTITION_STYLE_GPT;
    create.Anonymous.Gpt = CREATE_DISK_GPT {
        DiskId: to_guid(disk_id),
        MaxPartitionCount: 128,
    };
    ioctl(
        &handle,
        IOCTL_DISK_CREATE_DISK,
        &create as *const _ as *const c_void,
        mem::size_of::<CREATE_DISK>(),
        ptr::null_mut(),
        0,
    )
    .map_err(|e| format!("cannot create the GPT (error {e})"))?;

    let base = mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
    let step = mem::size_of::<PARTITION_INFORMATION_EX>();
    let mut buf = vec![0u8; base + step * parts.len()];
    let mut head: DRIVE_LAYOUT_INFORMATION_EX = unsafe { mem::zeroed() };
    head.PartitionStyle = PARTITION_STYLE_GPT as u32;
    head.PartitionCount = parts.len() as u32;
    head.Anonymous.Gpt = DRIVE_LAYOUT_INFORMATION_GPT {
        DiskId: to_guid(disk_id),
        StartingUsableOffset: 1 << 20,
        UsableLength: bytes as i64 - (2 << 20),
        MaxPartitionCount: 128,
    };
    unsafe {
        ptr::copy_nonoverlapping(
            &head as *const _ as *const u8,
            buf.as_mut_ptr(),
            mem::size_of::<DRIVE_LAYOUT_INFORMATION_EX>().min(base),
        );
    }
    for (i, p) in parts.iter().enumerate() {
        let mut entry: PARTITION_INFORMATION_EX = unsafe { mem::zeroed() };
        entry.PartitionStyle = PARTITION_STYLE_GPT;
        entry.StartingOffset = p.offset as i64;
        entry.PartitionLength = p.length as i64;
        entry.PartitionNumber = i as u32 + 1;
        entry.RewritePartition = true;
        let mut name = [0u16; 36];
        for (slot, ch) in name.iter_mut().zip(p.name.encode_utf16()) {
            *slot = ch;
        }
        entry.Anonymous.Gpt = PARTITION_INFORMATION_GPT {
            PartitionType: to_guid(p.kind),
            PartitionId: to_guid(random_guid()),
            Attributes: 0,
            Name: name,
        };
        unsafe {
            ptr::copy_nonoverlapping(
                &entry as *const _ as *const u8,
                buf.as_mut_ptr().add(base + i * step),
                step,
            );
        }
    }
    ioctl(
        &handle,
        IOCTL_DISK_SET_DRIVE_LAYOUT_EX,
        buf.as_ptr() as *const c_void,
        buf.len(),
        ptr::null_mut(),
        0,
    )
    .map_err(|e| format!("cannot write the partition table (error {e})"))?;
    let _ = ioctl(&handle, IOCTL_DISK_UPDATE_PROPERTIES, ptr::null(), 0, ptr::null_mut(), 0);
    Ok(())
}

pub fn lock_volume(volume: &str) -> Result<Handle, String> {
    let handle = open(volume.trim_end_matches('\\'), GENERIC_READ | GENERIC_WRITE)?;
    ioctl(&handle, FSCTL_LOCK_VOLUME, ptr::null(), 0, ptr::null_mut(), 0)
        .map_err(|e| format!("cannot lock the volume (error {e})"))?;
    ioctl(&handle, FSCTL_DISMOUNT_VOLUME, ptr::null(), 0, ptr::null_mut(), 0)
        .map_err(|e| format!("cannot dismount the volume (error {e})"))?;
    Ok(handle)
}
