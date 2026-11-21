pub struct Seg {
    pub name: String,
    pub vm: u64,
    pub vsize: u64,
    pub off: u64,
    pub fsize: u64,
}

pub struct Image {
    pub segs: Vec<Seg>,
    pub entry: u64,
    pub chain_starts: Vec<u64>,
    pub chain_format: u32,
}

fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u64le(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

fn name(d: &[u8], o: usize) -> String {
    let raw = &d[o..o + 16];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(16);
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

pub fn parse(d: &[u8]) -> Image {
    assert_eq!(u32le(d, 0), 0xfeedfacf, "not a 64-bit mach-o");
    let ncmds = u32le(d, 16) as usize;
    let mut off = 32;
    let mut segs = Vec::new();
    let mut entry = 0;
    let mut chain_starts = Vec::new();
    let mut chain_format = 0;
    for _ in 0..ncmds {
        let cmd = u32le(d, off);
        let size = u32le(d, off + 4) as usize;
        if cmd == 0x19 {
            let seg = Seg {
                name: name(d, off + 8),
                vm: u64le(d, off + 24),
                vsize: u64le(d, off + 32),
                off: u64le(d, off + 40),
                fsize: u64le(d, off + 48),
            };
            let nsects = u32le(d, off + 64) as usize;
            for s in 0..nsects {
                let so = off + 72 + s * 80;
                if name(d, so) == "__chain_starts" {
                    let foff = u32le(d, so + 48) as usize;
                    chain_format = u32le(d, foff);
                    let n = u32le(d, foff + 4) as usize;
                    for i in 0..n {
                        chain_starts.push(u32le(d, foff + 8 + i * 4) as u64);
                    }
                }
            }
            segs.push(seg);
        } else if cmd == 0x5 {
            entry = u64le(d, off + 16 + 32 * 8);
        }
        off += size;
    }
    Image { segs, entry, chain_starts, chain_format }
}

impl Image {
    pub fn lo(&self) -> u64 {
        self.segs.iter().filter(|s| s.vsize > 0).map(|s| s.vm).min().unwrap()
    }

    pub fn hi(&self) -> u64 {
        self.segs.iter().filter(|s| s.vsize > 0).map(|s| s.vm + s.vsize).max().unwrap()
    }

    pub fn va_of_fileoff(&self, fo: u64) -> Option<u64> {
        self.segs.iter().find(|s| s.fsize > 0 && fo >= s.off && fo < s.off + s.fsize).map(|s| s.vm + (fo - s.off))
    }
}
