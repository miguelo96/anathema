use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use uefi::boot::{self, ScopedProtocol};
use uefi::proto::media::block::BlockIO;

struct Vol {
    bio: ScopedProtocol<BlockIO>,
    media_id: u32,
    block: usize,
    fat_off: u64,
    heap_off: u64,
    cluster_shift: u32,
    bps_shift: u32,
    root: u32,
    clusters: u32,
}

impl Vol {
    fn read(&mut self, byte_off: u64, out: &mut [u8]) -> bool {
        let block = self.block as u64;
        let first = byte_off / block;
        let last = (byte_off + out.len() as u64 + block - 1) / block;
        let mut tmp = vec![0u8; ((last - first) * block) as usize];
        if self.bio.read_blocks(self.media_id, first, &mut tmp).is_err() {
            return false;
        }
        let s = (byte_off - first * block) as usize;
        out.copy_from_slice(&tmp[s..s + out.len()]);
        true
    }

    fn cluster_off(&self, c: u32) -> u64 {
        self.heap_off + ((c as u64 - 2) << self.cluster_shift)
    }

    fn fat_next(&mut self, c: u32) -> Option<u32> {
        let mut b = [0u8; 4];
        if !self.read(self.fat_off + c as u64 * 4, &mut b) {
            return None;
        }
        Some(u32::from_le_bytes(b))
    }

    fn read_chain(&mut self, first: u32, len: u64, contiguous: bool) -> Option<Vec<u8>> {
        let csize = 1u64 << self.cluster_shift;
        let mut out = Vec::new();
        out.try_reserve_exact(len as usize).ok()?;
        out.resize(len as usize, 0);
        let mut done = 0u64;
        let mut c = first;
        while done < len {
            let n = (len - done).min(csize);
            if contiguous {
                let run = ((len - done) + csize - 1) / csize;
                let bytes = (len - done).min(run * csize).min(64 << 20);
                let off = self.cluster_off(c);
                if !self.read(off, &mut out[done as usize..(done + bytes) as usize]) {
                    return None;
                }
                let used = (bytes + csize - 1) / csize;
                done += bytes;
                c += used as u32;
            } else {
                if c < 2 || c >= self.clusters + 2 {
                    return None;
                }
                let off = self.cluster_off(c);
                if !self.read(off, &mut out[done as usize..(done + n) as usize]) {
                    return None;
                }
                done += n;
                if done < len {
                    c = self.fat_next(c)?;
                }
            }
        }
        Some(out)
    }

    fn find(&mut self, dir_first: u32, dir_len: Option<u64>, contiguous: bool, name: &str) -> Option<(u32, u64, bool, bool)> {
        let data = match dir_len {
            Some(l) => self.read_chain(dir_first, l, contiguous)?,
            None => {
                let mut all = Vec::new();
                let mut c = dir_first;
                let csize = 1usize << self.cluster_shift;
                for _ in 0..4096 {
                    let mut buf = vec![0u8; csize];
                    if !self.read(self.cluster_off(c), &mut buf) {
                        return None;
                    }
                    all.extend_from_slice(&buf);
                    c = self.fat_next(c)?;
                    if c < 2 || c >= 0xffff_fff8 {
                        break;
                    }
                }
                all
            }
        };
        let mut i = 0;
        while i + 32 <= data.len() {
            let t = data[i];
            if t == 0 {
                break;
            }
            if t == 0x85 {
                let secondary = data[i + 1] as usize;
                let attrs = u16::from_le_bytes([data[i + 4], data[i + 5]]);
                let mut first = 0u32;
                let mut size = 0u64;
                let mut flags = 0u8;
                let mut nm = String::new();
                for k in 1..=secondary {
                    let e = i + 32 * k;
                    if e + 32 > data.len() {
                        break;
                    }
                    match data[e] {
                        0xc0 => {
                            flags = data[e + 1];
                            size = u64::from_le_bytes(data[e + 24..e + 32].try_into().unwrap());
                            first = u32::from_le_bytes(data[e + 20..e + 24].try_into().unwrap());
                        }
                        0xc1 => {
                            for j in 0..15 {
                                let u = u16::from_le_bytes([data[e + 2 + j * 2], data[e + 3 + j * 2]]);
                                if u == 0 {
                                    break;
                                }
                                nm.push(char::from_u32(u as u32).unwrap_or('?'));
                            }
                        }
                        _ => {}
                    }
                }
                if nm.eq_ignore_ascii_case(name) {
                    return Some((first, size, flags & 2 != 0, attrs & 0x10 != 0));
                }
                i += 32 * (secondary + 1);
                continue;
            }
            i += 32;
        }
        None
    }
}

pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    let handles = boot::find_handles::<BlockIO>().map_err(|e| alloc::format!("no block devices: {:?}", e.status()))?;
    for h in handles {
        let bio = match boot::open_protocol_exclusive::<BlockIO>(h) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let media = bio.media();
        if !media.is_logical_partition() || !media.is_media_present() {
            continue;
        }
        let block = media.block_size() as usize;
        let media_id = media.media_id();
        let mut v = Vol { bio, media_id, block, fat_off: 0, heap_off: 0, cluster_shift: 0, bps_shift: 0, root: 0, clusters: 0 };
        let mut bs = vec![0u8; 512];
        if !v.read(0, &mut bs) || &bs[3..11] != b"EXFAT   " {
            continue;
        }
        v.bps_shift = bs[0x6c] as u32;
        v.cluster_shift = v.bps_shift + bs[0x6d] as u32;
        v.fat_off = (u32::from_le_bytes(bs[0x50..0x54].try_into().unwrap()) as u64) << v.bps_shift;
        v.heap_off = (u32::from_le_bytes(bs[0x58..0x5c].try_into().unwrap()) as u64) << v.bps_shift;
        v.clusters = u32::from_le_bytes(bs[0x5c..0x60].try_into().unwrap());
        v.root = u32::from_le_bytes(bs[0x60..0x64].try_into().unwrap());
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let mut dir = v.root;
        let mut dir_len: Option<u64> = None;
        let mut contig = false;
        for (idx, part) in parts.iter().enumerate() {
            let (first, size, nofat, is_dir) = match v.find(dir, dir_len, contig, part) {
                Some(x) => x,
                None => return Err(alloc::format!("{} not found on exfat volume", part)),
            };
            if idx + 1 == parts.len() {
                return v.read_chain(first, size, nofat).ok_or_else(|| String::from("read error on exfat volume"));
            }
            if !is_dir {
                return Err(alloc::format!("{} is not a directory", part));
            }
            dir = first;
            dir_len = Some(size);
            contig = nofat;
        }
    }
    Err(String::from("no exfat volume found"))
}
