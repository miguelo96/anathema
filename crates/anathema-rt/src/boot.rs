use crate::dt;
use crate::macho::Image;
use crate::mem::{Ram, RAM_BASE, RAM_SIZE};

pub const KERNEL_PA: u64 = RAM_BASE + 0x0300_4000;

pub struct Loaded {
    pub entry_pa: u64,
    pub args_pa: u64,
    pub virt_base: u64,
}

fn align(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

pub fn load_kernel(ram: &Ram, img: &Image, data: &[u8], dt_bytes: &[u8], cmdline: &str) -> Loaded {
    let virt_base = img.lo();
    let span = img.hi() - virt_base;
    for s in &img.segs {
        if s.vsize == 0 {
            continue;
        }
        let dst = KERNEL_PA + (s.vm - virt_base);
        let z = ram.bytes(dst, s.vsize as usize);
        z.fill(0);
        if s.fsize > 0 {
            z[..s.fsize as usize].copy_from_slice(&data[s.off as usize..(s.off + s.fsize) as usize]);
        }
    }
    let mut applied = 0usize;
    let mut bad = 0usize;
    for c in &img.chains {
        for (i, st) in c.starts.iter().enumerate() {
            if *st == 0xffff {
                continue;
            }
            let mut va = virt_base + c.seg_off + i as u64 * c.page_size + (*st as u64 & 0x7fff);
            loop {
                let pa = KERNEL_PA + (va - virt_base);
                let v = ram.read(pa, 8);
                let (next, new) = match c.format {
                    7 | 8 => ((v >> 51) & 0xfff, virt_base + (v & 0x3fff_ffff)),
                    f => panic!("unsupported chained pointer format {}", f),
                };
                if new < virt_base || new >= virt_base + span {
                    bad += 1;
                }
                ram.write(pa, 8, new);
                applied += 1;
                if next == 0 {
                    break;
                }
                va += next * 4;
            }
        }
    }
    println!("kernel: virt base {:#x}, span {:#x}, entry va {:#x}, {} fixups ({} out of range)", virt_base, span, img.entry, applied, bad);

    let mut root = dt::parse(dt_bytes);
    if let Some(mem) = root.child_mut("memory") {
        let mut v = Vec::new();
        v.extend_from_slice(&RAM_BASE.to_le_bytes());
        v.extend_from_slice(&RAM_SIZE.to_le_bytes());
        mem.set("reg", v);
    }
    let mut seed = vec![0u8; 256];
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for b in seed.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    if let Some(chosen) = root.child_mut("chosen") {
        chosen.set("random-seed", seed);
        let mut b = cmdline.as_bytes().to_vec();
        b.push(0);
        chosen.set("boot-args", b);
        chosen.set("kaslr-seed", vec![0u8; 8]);
    }
    let dt_final = dt::serialize(&root);
    let dt_pa = align(KERNEL_PA + span, 0x4000);
    ram.bytes(dt_pa, dt_final.len()).copy_from_slice(&dt_final);
    let args_pa = align(dt_pa + dt_final.len() as u64, 0x4000);
    let top = align(args_pa + 0x4000, 0x4000);
    let args = ram.bytes(args_pa, 0x400);
    args.fill(0);
    let put16 = |a: &mut [u8], o: usize, v: u16| a[o..o + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |a: &mut [u8], o: usize, v: u32| a[o..o + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |a: &mut [u8], o: usize, v: u64| a[o..o + 8].copy_from_slice(&v.to_le_bytes());
    put16(args, 0, 2);
    put16(args, 2, 2);
    put64(args, 8, virt_base);
    put64(args, 0x10, KERNEL_PA);
    put64(args, 0x18, RAM_SIZE - (KERNEL_PA - RAM_BASE));
    put64(args, 0x20, top);
    put32(args, 0x58, 0);
    put64(args, 0x60, virt_base + (dt_pa - KERNEL_PA));
    put32(args, 0x68, dt_final.len() as u32);
    let cl = cmdline.as_bytes();
    args[0x70..0x70 + cl.len().min(607)].copy_from_slice(&cl[..cl.len().min(607)]);
    put64(args, 0x2d0, 0);
    put64(args, 0x2d8, RAM_SIZE);
    println!("devicetree at {:#x} ({} bytes), boot args at {:#x}, top of kernel data {:#x}", dt_pa, dt_final.len(), args_pa, top);
    Loaded { entry_pa: KERNEL_PA + (img.entry - virt_base), args_pa, virt_base }
}
