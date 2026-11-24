#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::fs::FileSystem;
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::{cstr16, println};

const KBASE: u64 = 0xffff_ff80_0000_0000;
const DEFAULT_ARGS: &str = "-v serial=3 debug=0x14e cpus=1 kt_fixed_zones=200 kt_var_heaps=6";

struct Patch {
    va: u64,
    expect: &'static [u8],
    replace: &'static [u8],
    why: &'static str,
}

// applied only when \\anathema\\xpatch-tsc exists: qemu's software cpu reads MSR_PLATFORM_INFO as zero,
// which makes tsc_init divide by a zero bus ratio. use ratio 24 (2.4 GHz tsc on a 100 MHz bus).
const TSC_PATCHES: &[Patch] = &[Patch {
    va: 0xffff_ff80_0040_198d,
    expect: &[0x48, 0x8b, 0x0d, 0xdc, 0x65, 0xbf, 0x00],
    replace: &[0xb9, 0x18, 0x00, 0x00, 0x00, 0x90, 0x90],
    why: "tsc_init bus ratio",
}];


// always applied: the stock macOS 27 x86_64 kernel runs its early memory reservations before
// virtual_space_start/end are set (they stay 0), so the first reserved range is mapped at address 0.
// the stub asks pmap_virtual_space() for the real range first, then continues into the original code.
const EARLY_VM_PATCHES: &[Patch] = &[
    Patch {
        va: 0xffff_ff80_00b0_3afd,
        expect: &[0x55, 0x48, 0x89, 0xe5],
        replace: &[0xc3, 0x90, 0x90, 0x90],
        why: "skip exe boothash salt check (needs corecrypto kext, no boot kc yet)",
    },
    Patch {
        va: 0xffffff8000c42700,
        expect: &[0u8; 37],
        replace: &[0x50, 0x48, 0x8d, 0x3d, 0xf8, 0x3e, 0x29, 0x00, 0x48, 0x8d, 0x35, 0xf9, 0x3e, 0x29, 0x00, 0xe8, 0x6c, 0x2e, 0x79, 0xff, 0x58, 0x48, 0xc7, 0x05, 0x68, 0x77, 0x58, 0x00, 0x00, 0x40, 0x00, 0x00, 0xe9, 0xe8, 0xd2, 0xeb, 0xff],
        why: "early vm stub",
    },
    Patch {
        va: 0xffffff8000affa02,
        expect: &[0x48, 0xc7, 0x05, 0x7b, 0xa4],
        replace: &[0xe9, 0xf9, 0x2c, 0x14, 0x00],
        why: "early vm entry",
    },
    Patch {
        va: 0xffff_ff80_00af_e504,
        expect: &[0x48, 0x89, 0xde, 0x4c, 0x89, 0xf2],
        replace: &[0x4c, 0x89, 0xf6, 0x48, 0x89, 0xda],
        why: "early kernel map range bounds",
    },
    Patch {
        va: 0xffffff8000c42800,
        expect: &[0u8; 197],
        replace: &[0x57, 0x56, 0x52, 0x51, 0x41, 0x50, 0x41, 0x51, 0x50, 0x41, 0x52, 0x41, 0x53, 0x48, 0x8d, 0x05, 0x1c, 0x00, 0x00, 0x00, 0xff, 0xd0, 0x41, 0x5b, 0x41, 0x5a, 0x58, 0x41, 0x59, 0x41, 0x58, 0x59, 0x5a, 0x5e, 0x5f, 0x48, 0xb8, 0xe0, 0xe6, 0x3d, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff, 0xe0, 0x00, 0x48, 0x89, 0xd0, 0x48, 0x29, 0xf0, 0x0f, 0x96, 0xc1, 0x48, 0x3d, 0x01, 0x00, 0x00, 0x10, 0x0f, 0x93, 0xc0, 0x08, 0xc8, 0x75, 0x7e, 0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x50, 0x48, 0x89, 0xd3, 0x49, 0x89, 0xf6, 0x49, 0x89, 0xff, 0x48, 0xb8, 0x30, 0xaa, 0x3d, 0x00, 0x80, 0xff, 0xff, 0xff, 0x4c, 0x8b, 0xa8, 0xd8, 0xd2, 0xc1, 0x00, 0x4c, 0x8b, 0xa0, 0xe0, 0xd2, 0xc1, 0x00, 0x49, 0x81, 0xe6, 0x00, 0xf0, 0xff, 0xff, 0xeb, 0x0e, 0x66, 0x90, 0x49, 0x81, 0xc6, 0x00, 0x10, 0x00, 0x00, 0x49, 0x39, 0xde, 0x73, 0x2a, 0x4c, 0x89, 0xff, 0x4c, 0x89, 0xf6, 0x48, 0xb8, 0x30, 0xaa, 0x3d, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff, 0xd0, 0x48, 0x3d, 0x00, 0x10, 0x00, 0x00, 0x72, 0xda, 0x48, 0xc1, 0xe8, 0x0c, 0x4c, 0x39, 0xe8, 0x77, 0xd1, 0x41, 0x80, 0x24, 0x04, 0xfe, 0xeb, 0xca, 0x48, 0x83, 0xc4, 0x08, 0x5b, 0x41, 0x5c, 0x41, 0x5d, 0x41, 0x5e, 0x41, 0x5f, 0x5d, 0xc3],
        why: "early pv repair code",
    },
    Patch {
        va: 0xffffff8000afbc36,
        expect: &[0xe8, 0xa5, 0x2a, 0x8e, 0xff],
        replace: &[0xe8, 0xc5, 0x6b, 0x14, 0x00],
        why: "early pv repair hook",
    },
    Patch {
        va: 0xffff_ff80_00ab_5f10,
        expect: &[0xb8, 0x02, 0x00, 0x00, 0x00],
        replace: &[0xb8, 0x01, 0x00, 0x00, 0x00],
        why: "standalone kernel counts as a static collection",
    },
    Patch {
        va: 0xffff_ff80_0020_0018,
        expect: &[0x01, 0x80, 0x21, 0x00],
        replace: &[0x01, 0x80, 0x21, 0x80],
        why: "kernel header says it is in a collection",
    },
];

struct Seg {
    pa: u64,
    vmsize: u64,
    off: u64,
    fsize: u64,
}

struct Kernel {
    segs: Vec<Seg>,
    entry_pa: u64,
    end_pa: u64,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

fn parse(k: &[u8]) -> Result<Kernel, &'static str> {
    if k.len() < 32 || rd32(k, 0) != 0xfeed_facf {
        return Err("not a 64-bit mach-o");
    }
    if rd32(k, 4) != 0x0100_0007 {
        return Err("not an x86_64 kernel");
    }
    let ncmds = rd32(k, 16) as usize;
    let mut off = 32usize;
    let mut segs = Vec::new();
    let mut entry_va = 0u64;
    let mut end_pa = 0u64;
    for _ in 0..ncmds {
        let cmd = rd32(k, off);
        let size = rd32(k, off + 4) as usize;
        if cmd == 0x19 {
            let vmaddr = rd64(k, off + 24);
            let vmsize = rd64(k, off + 32);
            let fileoff = rd64(k, off + 40);
            let filesize = rd64(k, off + 48);
            if vmsize != 0 {
                if vmaddr < KBASE {
                    return Err("segment below the kernel base");
                }
                let pa = vmaddr - KBASE;
                end_pa = end_pa.max(pa + vmsize);
                segs.push(Seg { pa, vmsize, off: fileoff, fsize: filesize });
            }
        } else if cmd == 0x5 {
            entry_va = rd64(k, off + 16 + 16 * 8);
        }
        off += size;
    }
    if entry_va < KBASE {
        return Err("no entry point");
    }
    Ok(Kernel { segs, entry_pa: entry_va - KBASE, end_pa })
}

fn align_up(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

fn node(props: &[(&str, &[u8])], children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(props.len() as u32).to_le_bytes());
    out.extend_from_slice(&(children.len() as u32).to_le_bytes());
    for (name, value) in props {
        let mut n = [0u8; 32];
        let nb = name.as_bytes();
        n[..nb.len()].copy_from_slice(nb);
        out.extend_from_slice(&n);
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    for c in children {
        out.extend_from_slice(c);
    }
    out
}

#[target_feature(enable = "rdrand")]
unsafe fn rdrand64() -> u64 {
    let mut v = 0u64;
    for _ in 0..16 {
        if core::arch::x86_64::_rdrand64_step(&mut v) == 1 {
            return v;
        }
    }
    core::arch::x86_64::_rdtsc()
}

fn random_seed() -> [u8; 64] {
    let mut out = [0u8; 64];
    let mut x = unsafe { core::arch::x86_64::_rdtsc() } | 1;
    for i in 0..8 {
        let r = unsafe { rdrand64() };
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out[i * 8..i * 8 + 8].copy_from_slice(&(r ^ x).to_le_bytes());
    }
    out
}

fn device_tree() -> Vec<u8> {
    let memory_map = node(&[("name", b"memory-map\0")], &[]);
    let seed = random_seed();
    let chosen = node(&[("name", b"chosen\0"), ("random-seed", &seed)], &[memory_map]);
    let fsb = 100_000_000u64.to_le_bytes();
    let platform = node(&[("name", b"platform\0"), ("FSBFrequency", &fsb)], &[]);
    let efi = node(&[("name", b"efi\0")], &[platform]);
    node(&[("name", b"device-tree\0")], &[chosen, efi])
}

// a minimal MH_FILESET kernel collection header: one segment covering the kernel and one entry, the kernel itself.
// the kernel only needs this to take its "boot kernel collection" paths; there are no kexts in it yet.
// set to true to list the kernel itself as a collection entry (it gets counted twice by kalloc, so off for now)
fn std_entry() -> bool {
    true
}

fn fake_kc() -> Vec<u8> {
    let mut h: Vec<u8> = Vec::new();
    let w32 = |h: &mut Vec<u8>, v: u32| h.extend_from_slice(&v.to_le_bytes());
    let w64 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&v.to_le_bytes());
    w32(&mut h, 0xfeed_facf);
    w32(&mut h, 0x0100_0007);
    w32(&mut h, 3);
    w32(&mut h, 0xc);
    w32(&mut h, 3);
    w32(&mut h, 72 + 152 + 56);
    w32(&mut h, 0);
    w32(&mut h, 0);
    // LC_SEGMENT_64 __TEXT
    w32(&mut h, 0x19);
    w32(&mut h, 72);
    let mut name = [0u8; 16];
    name[..6].copy_from_slice(b"__TEXT");
    h.extend_from_slice(&name);
    w64(&mut h, KBASE + 0x20_0000);
    w64(&mut h, 0x105_2000);
    w64(&mut h, 0);
    w64(&mut h, 0);
    w32(&mut h, 5);
    w32(&mut h, 5);
    w32(&mut h, 0);
    w32(&mut h, 0);
    // LC_SEGMENT_64 __PRELINK_INFO with an empty __info section: the kernel looks it up without a null check
    w32(&mut h, 0x19);
    w32(&mut h, 72 + 80);
    let mut pn = [0u8; 16];
    pn[..14].copy_from_slice(b"__PRELINK_INFO");
    h.extend_from_slice(&pn);
    w64(&mut h, KBASE + 0x20_0000 + 0x105_2000);
    w64(&mut h, 0);
    w64(&mut h, 0);
    w64(&mut h, 0);
    w32(&mut h, 3);
    w32(&mut h, 3);
    w32(&mut h, 1);
    w32(&mut h, 0);
    let mut sn = [0u8; 16];
    sn[..6].copy_from_slice(b"__info");
    h.extend_from_slice(&sn);
    h.extend_from_slice(&pn);
    w64(&mut h, KBASE + 0x20_0000 + 0x105_2000);
    w64(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    w32(&mut h, 0);
    // LC_FILESET_ENTRY com.apple.kernel
    w32(&mut h, 0x8000_0035);
    w32(&mut h, 56);
    w64(&mut h, KBASE + 0x20_0000);
    w64(&mut h, 0);
    w32(&mut h, 32);
    w32(&mut h, 0);
    let mut id = [0u8; 24];
    id[..16].copy_from_slice(b"com.apple.kernel");
    h.extend_from_slice(&id);
    h
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    !c
}

// a copy of the firmware system table with no runtime services, so the kernel only reads the acpi table list
fn fake_system_table(at: u64) -> Vec<u8> {
    let mut out = vec![0u8; 0x2000];
    if let Some(st) = uefi::table::system_table_raw() {
        let src = st.as_ptr() as *const u8;
        unsafe {
            let hs = core::ptr::read_unaligned(src.add(12) as *const u32) as usize;
            core::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), hs.min(0x100));
            let n = core::ptr::read_unaligned(src.add(104) as *const u64) as usize;
            let cfg = core::ptr::read_unaligned(src.add(112) as *const u64) as *const u8;
            let bytes = (n * 24).min(0x800);
            core::ptr::copy_nonoverlapping(cfg, out.as_mut_ptr().add(0x800), bytes);
            out[88..96].copy_from_slice(&0u64.to_le_bytes());
            out[104..112].copy_from_slice(&((bytes / 24) as u64).to_le_bytes());
            out[112..120].copy_from_slice(&(at + 0x800).to_le_bytes());
            out[16..20].copy_from_slice(&0u32.to_le_bytes());
            let c = crc32(&out[..hs]);
            out[16..20].copy_from_slice(&c.to_le_bytes());
        }
    }
    out
}

fn com1_byte(b: u8) {
    unsafe {
        loop {
            let status: u8;
            asm!("in al, dx", out("al") status, in("dx") 0x3fdu16, options(nomem, nostack));
            if status & 0x20 != 0 {
                break;
            }
        }
        asm!("out dx, al", in("dx") 0x3f8u16, in("al") b, options(nomem, nostack));
    }
}

fn com1(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            com1_byte(b'\r');
        }
        com1_byte(b);
    }
}

fn pages(bytes: u64) -> usize {
    (align_up(bytes, 4096) / 4096) as usize
}

unsafe fn handoff(pml4: u64, gdtr: u64, entry: u32, bootargs: u32) -> ! {
    asm!(
        "cli",
        "mov cr3, {pml4}",
        "lgdt [{gdtr}]",
        "lea {t}, [rip + 2f]",
        "push 0x08",
        "push {t}",
        "retfq",
        ".code32",
        "2:",
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "mov fs, ax",
        "mov gs, ax",
        "mov eax, cr0",
        "and eax, 0x7fffffff",
        "mov cr0, eax",
        "mov ecx, 0xC0000080",
        "rdmsr",
        "and eax, 0xfffffeff",
        "wrmsr",
        "mov eax, cr4",
        "and eax, 0xffffffdf",
        "mov cr4, eax",
        "mov eax, edi",
        "jmp esi",
        ".code64",
        pml4 = in(reg) pml4,
        gdtr = in(reg) gdtr,
        t = out(reg) _,
        in("rsi") entry as u64,
        in("rdi") bootargs as u64,
        options(nostack)
    );
    loop {}
}

fn fail(msg: &str) -> Status {
    println!("anathema-xboot: {}", msg);
    com1("anathema-xboot: ");
    com1(msg);
    com1("\n");
    boot::stall(10_000_000);
    Status::LOAD_ERROR
}

#[entry]
fn main() -> Status {
    let _ = boot::set_watchdog_timer(0, 0x10000, None);
    println!("anathema-xboot: native x86_64 macOS kernel loader");
    com1("anathema-xboot: start\n");

    let fs_proto = match boot::get_image_file_system(boot::image_handle()) {
        Ok(p) => p,
        Err(_) => return fail("cannot open the boot file system"),
    };
    let mut fs = FileSystem::new(fs_proto);
    let kernel_file: Vec<u8> = match fs.read(cstr16!("\\anathema\\xkernel")) {
        Ok(d) => d,
        Err(_) => return fail("cannot read \\anathema\\xkernel"),
    };
    let args: String = match fs.read(cstr16!("\\anathema\\xargs.txt")) {
        Ok(d) => String::from_utf8_lossy(&d).trim().into(),
        Err(_) => String::from(DEFAULT_ARGS),
    };
    let want_tsc_patch = fs.read(cstr16!("\\anathema\\xpatch-tsc")).is_ok();
    let kern = match parse(&kernel_file) {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    println!("kernel: {} segments, entry {:#x}, image end {:#x}", kern.segs.len(), kern.entry_pa, kern.end_pa);

    let base = align_up(kern.end_pa, 4096);
    let dt = device_tree();
    let boot_args_pa = base;
    let dt_pa = base + 0x1000;
    let mmap_pa = align_up(dt_pa + dt.len() as u64, 0x1000);
    let mmap_cap = 0x10000u64;
    let kc_pa = mmap_pa + mmap_cap;
    let kc = fake_kc();
    let est_pa = kc_pa + 0x1000;
    let est = fake_system_table(est_pa);
    let final_end = kc_pa + 0x3000;
    let kaddr = 0x100000u64;
    println!("boot data: args {:#x}, tree {:#x} ({} bytes), map {:#x}, end {:#x}", boot_args_pa, dt_pa, dt.len(), mmap_pa, final_end);

    let src_lo = kernel_file.as_ptr() as u64;
    let src_hi = src_lo + kernel_file.len() as u64;
    if src_lo < final_end && src_hi > kaddr {
        return fail("the kernel file buffer overlaps the load area");
    }

    // identity map of the first 4 GB with 2 MB pages, plus a small GDT, both below 4 GB
    let table_ptr = match boot::allocate_pages(AllocateType::MaxAddress(0xffff_ffff), MemoryType::LOADER_DATA, 7) {
        Ok(p) => p.as_ptr(),
        Err(_) => return fail("cannot allocate page tables"),
    };
    let tables = table_ptr as u64;
    if tables < final_end {
        return fail("page tables landed inside the load area");
    }
    unsafe {
        core::ptr::write_bytes(table_ptr, 0, 7 * 4096);
        let pml4 = table_ptr as *mut u64;
        let pdpt = table_ptr.add(4096) as *mut u64;
        *pml4 = (tables + 4096) | 3;
        for g in 0..4u64 {
            *pdpt.add(g as usize) = (tables + 8192 + g * 4096) | 3;
            let pd = table_ptr.add(8192 + g as usize * 4096) as *mut u64;
            for i in 0..512u64 {
                *pd.add(i as usize) = (g * 0x4000_0000 + i * 0x20_0000) | 0x83;
            }
        }
        let gdt = table_ptr.add(6 * 4096) as *mut u64;
        *gdt.add(0) = 0;
        *gdt.add(1) = 0x00cf_9a00_0000_ffff;
        *gdt.add(2) = 0x00cf_9200_0000_ffff;
        let gdtr = table_ptr.add(6 * 4096 + 0x40);
        *(gdtr as *mut u16) = 23;
        *(gdtr.add(2) as *mut u64) = tables + 6 * 4096;
    }
    let pml4_pa = tables;
    let gdtr_pa = tables + 6 * 4096 + 0x40;

    // boot_args staged in memory, patched with the memory map after boot services end
    let mut ba = vec![0u8; 4096];
    ba[0..2].copy_from_slice(&1u16.to_le_bytes());
    ba[2..4].copy_from_slice(&2u16.to_le_bytes());
    ba[4] = 64;
    let cl = args.as_bytes();
    let n = cl.len().min(1023);
    ba[8..8 + n].copy_from_slice(&cl[..n]);
    ba[1072..1076].copy_from_slice(&(dt_pa as u32).to_le_bytes());
    ba[1076..1080].copy_from_slice(&(dt.len() as u32).to_le_bytes());
    ba[1080..1084].copy_from_slice(&(kaddr as u32).to_le_bytes());
    ba[1084..1088].copy_from_slice(&((final_end - kaddr) as u32).to_le_bytes());
    ba[1152..1160].copy_from_slice(&100_000_000u64.to_le_bytes());
    ba[1256..1264].copy_from_slice(&(KBASE + kc_pa).to_le_bytes());
    ba[1104..1108].copy_from_slice(&((est_pa) as u32).to_le_bytes());

    if !args.is_empty() {
        println!("command line: {}", args);
    }
    println!("leaving boot services");
    com1("anathema-xboot: exiting boot services\n");

    let mmap = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };
    let raw = mmap.buffer();
    let meta = mmap.meta();
    let mut phys: u64 = 0;
    for d in mmap.entries() {
        let t = d.ty.0;
        if (1..=7).contains(&t) || t == 9 {
            phys += d.page_count * 4096;
        }
    }
    let copy_len = raw.len().min(mmap_cap as usize);

    unsafe {
        for s in kern.segs.iter() {
            let dst = s.pa as *mut u8;
            let take = s.fsize.min(s.vmsize) as usize;
            core::ptr::copy_nonoverlapping(kernel_file.as_ptr().add(s.off as usize), dst, take);
            if s.vmsize as usize > take {
                core::ptr::write_bytes(dst.add(take), 0, s.vmsize as usize - take);
            }
        }
        let empty: &[Patch] = &[];
        let lists: [&[Patch]; 2] = [EARLY_VM_PATCHES, if want_tsc_patch { TSC_PATCHES } else { empty }];
        {
            for pt in lists.iter().flat_map(|l| l.iter()) {
                let dst = (pt.va - KBASE) as *mut u8;
                let mut same = true;
                for (i, b) in pt.expect.iter().enumerate() {
                    if *dst.add(i) != *b {
                        same = false;
                    }
                }
                if same {
                    for (i, b) in pt.replace.iter().enumerate() {
                        *dst.add(i) = *b;
                    }
                    com1("anathema-xboot: patched ");
                    com1(pt.why);
                    com1("\n");
                } else {
                    com1("anathema-xboot: patch skipped (bytes differ): ");
                    com1(pt.why);
                    com1("\n");
                }
            }
        }
        core::ptr::write_bytes(base as *mut u8, 0, (final_end - base) as usize);
        core::ptr::copy_nonoverlapping(dt.as_ptr(), dt_pa as *mut u8, dt.len());
        core::ptr::copy_nonoverlapping(kc.as_ptr(), kc_pa as *mut u8, kc.len());
        core::ptr::copy_nonoverlapping(est.as_ptr(), est_pa as *mut u8, est.len());
        core::ptr::copy_nonoverlapping(raw.as_ptr(), mmap_pa as *mut u8, copy_len);
        let mut o = 0usize;
        while o + 32 <= copy_len {
            let ap = (mmap_pa as usize + o + 32) as *mut u64;
            *ap &= !(1u64 << 63);
            o += meta.desc_size;
        }
        ba[1032..1036].copy_from_slice(&(mmap_pa as u32).to_le_bytes());
        ba[1036..1040].copy_from_slice(&(copy_len as u32).to_le_bytes());
        ba[1040..1044].copy_from_slice(&(meta.desc_size as u32).to_le_bytes());
        ba[1044..1048].copy_from_slice(&meta.desc_version.to_le_bytes());
        ba[1144..1152].copy_from_slice(&phys.to_le_bytes());
        core::ptr::copy_nonoverlapping(ba.as_ptr(), boot_args_pa as *mut u8, 4096);
    }
    com1("anathema-xboot: kernel copied, jumping\n");
    unsafe { handoff(pml4_pa, gdtr_pa, kern.entry_pa as u32, boot_args_pa as u32) }
}
