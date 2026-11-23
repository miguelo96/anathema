//! Host-side tests for the runtime's software MMU, guest memory access and device tree code.
//!
//! Guest RAM is a plain `Vec<u8>` handed to `Ram::from_raw`, so nothing here maps executable
//! memory or runs translated code. Runs on any host: `cargo test -p anathema-rt --lib --tests`.
//!
//! Page tables use the 16K granule with T0SZ = 17 (47-bit VA, 3 levels), like the macOS kernel.

use anathema_rt::dt;
use anathema_rt::machine::Machine;
use anathema_rt::mem::Ram;
use anathema_rt::mmu::Mmu;

const BASE: u64 = 0x7000_0000;
const L1: u64 = BASE; // level 1 table
const L2: u64 = BASE + 0x4000; // level 2 table
const L3: u64 = BASE + 0x8000; // level 3 table
const TCR_16K_47BIT: u64 = 17 | (2 << 14); // T0SZ = 17, TG0 = 16K

const VALID: u64 = 1;
const TABLE_OR_PAGE: u64 = 2;
const AF: u64 = 1 << 10;

/// Guest RAM backed by a Vec. The Vec must outlive the returned `Ram`.
fn ram(mem: &mut Vec<u8>) -> Ram {
    unsafe { Ram::from_raw(BASE, mem.len() as u64, mem.as_mut_ptr()) }
}

fn put64(mem: &mut [u8], pa: u64, v: u64) {
    let o = (pa - BASE) as usize;
    mem[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// L1[0] -> L2, L2[0] -> L3, and L3[i] -> pages[i].
fn tables(mem: &mut [u8], pages: &[u64]) {
    put64(mem, L1, L2 | VALID | TABLE_OR_PAGE);
    put64(mem, L2, L3 | VALID | TABLE_OR_PAGE);
    for (i, &pa) in pages.iter().enumerate() {
        put64(mem, L3 + 8 * i as u64, pa | VALID | TABLE_OR_PAGE | AF);
    }
}

fn mmu() -> Mmu {
    Mmu { sctlr: 1, tcr: TCR_16K_47BIT, ttbr0: L1, ttbr1: 0 }
}

// ---------------------------------------------------------------- MMU walker

#[test]
fn walk_to_a_16k_page() {
    let mut mem = vec![0u8; 0x10000];
    tables(&mut mem, &[BASE + 0xc000]);
    let r = ram(&mut mem);
    let (pa, flags) = mmu().translate(&r, 0x123).ok().expect("mapped");
    assert_eq!(pa, BASE + 0xc123);
    assert_eq!(flags >> 4, 3, "leaf level is reported in bits 4-5");
}

#[test]
fn level_2_block_maps_32mb() {
    let mut mem = vec![0u8; 0x10000];
    tables(&mut mem, &[]);
    put64(&mut mem, L2, 0x7200_0000 | VALID | AF); // block, not table
    let r = ram(&mut mem);
    let (pa, flags) = mmu().translate(&r, 0x12345).ok().expect("mapped");
    assert_eq!(pa, 0x7201_2345);
    assert_eq!(flags >> 4, 2);
}

#[test]
fn missing_access_flag_is_an_access_flag_fault() {
    let mut mem = vec![0u8; 0x10000];
    tables(&mut mem, &[]);
    put64(&mut mem, L3, (BASE + 0xc000) | VALID | TABLE_OR_PAGE); // no AF
    let r = ram(&mut mem);
    let f = mmu().translate(&r, 0x10).err().expect("must fault");
    assert_eq!((f.level, f.kind), (3, 1));
}

#[test]
fn invalid_descriptor_is_a_translation_fault_at_that_level() {
    let mut mem = vec![0u8; 0x10000];
    put64(&mut mem, L1, L2 | VALID | TABLE_OR_PAGE); // L2 stays all zero
    let r = ram(&mut mem);
    let f = mmu().translate(&r, 0x10).err().expect("must fault");
    assert_eq!((f.level, f.kind), (2, 0));
}

#[test]
fn mmu_off_is_identity() {
    let mut mem = vec![0u8; 0x10000];
    let r = ram(&mut mem);
    let m = Mmu { sctlr: 0, ..mmu() };
    assert_eq!(m.translate(&r, 0x1234).ok().map(|(pa, _)| pa), Some(0x1234));
}

// ---------------------------------------------------------------- guest loads and stores

/// VA page 0 -> PA BASE+0x10000, VA page 1 -> PA BASE+0x20000 (not physically adjacent).
fn split_machine(mem: &mut Vec<u8>) -> Machine {
    tables(mem, &[BASE + 0x10000, BASE + 0x20000]);
    let mut m = Machine::new(ram(mem));
    m.mmu = mmu();
    m
}

#[test]
fn aligned_loads_and_stores_go_through_the_page_tables() {
    let mut mem = vec![0u8; 0x30000];
    let mut m = split_machine(&mut mem);
    m.store(0x4008, 8, 0x1122_3344_5566_7788).unwrap();
    assert_eq!(m.load(0x4008, 8).unwrap(), 0x1122_3344_5566_7788);
    assert_eq!(m.load(0x4008, 1).unwrap(), 0x88);
    drop(m);
    assert_eq!(mem[0x20008], 0x88, "VA 0x4008 must land in the second mapped page");
}

#[test]
fn load_from_an_unmapped_page_faults() {
    let mut mem = vec![0u8; 0x30000];
    let mut m = split_machine(&mut mem);
    assert!(m.load(0x8000, 8).is_err()); // L3[2] is empty
}

/// Review finding: an unaligned access that crosses a 16K page boundary is translated using
/// only its first byte, so the bytes past the boundary come from the wrong physical page.
/// arm64 allows unaligned accesses to normal memory and translates every byte, and XNU relies
/// on that (memcpy, decompressors). This is a candidate cause of the launchd ENOEXEC.
#[test]
#[ignore = "known bug: page-crossing unaligned accesses (see review notes)"]
fn unaligned_load_across_a_page_boundary_reads_both_pages() {
    let mut mem = vec![0u8; 0x30000];
    mem[0x13ffc..0x14000].copy_from_slice(&[0x11, 0x12, 0x13, 0x14]); // end of VA page 0
    mem[0x20000..0x20004].copy_from_slice(&[0x21, 0x22, 0x23, 0x24]); // start of VA page 1
    let mut m = split_machine(&mut mem);
    assert_eq!(m.load(0x3ffc, 8).unwrap(), 0x2423_2221_1413_1211);
}

#[test]
#[ignore = "known bug: page-crossing unaligned accesses (see review notes)"]
fn unaligned_store_across_a_page_boundary_writes_both_pages() {
    let mut mem = vec![0u8; 0x30000];
    let mut m = split_machine(&mut mem);
    m.store(0x3ffe, 4, 0xaabb_ccdd).unwrap();
    drop(m);
    assert_eq!(&mem[0x13ffe..0x14000], &[0xdd, 0xcc]);
    assert_eq!(&mem[0x20000..0x20002], &[0xbb, 0xaa]);
    assert_eq!(&mem[0x14000..0x14002], &[0, 0], "must not spill into the physically adjacent page");
}

// ---------------------------------------------------------------- device tree

fn dt_prop(name: &str, value: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 32];
    v[..name.len()].copy_from_slice(name.as_bytes());
    v.extend_from_slice(&(value.len() as u32).to_le_bytes());
    v.extend_from_slice(value);
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

#[test]
fn device_tree_round_trips() {
    let mut bytes = 1u32.to_le_bytes().to_vec(); // one property
    bytes.extend_from_slice(&0u32.to_le_bytes()); // no children
    bytes.extend(dt_prop("name", b"cpu\0"));
    let node = dt::parse(&bytes);
    assert_eq!(node.name(), "cpu");
    assert_eq!(dt::serialize(&node), bytes);
}

#[test]
fn device_tree_set_adds_and_replaces_properties() {
    let mut bytes = 1u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend(dt_prop("name", b"chosen\0"));
    let mut node = dt::parse(&bytes);
    node.set("boot-args", b"-v\0".to_vec());
    node.set("name", b"chosen2\0".to_vec());
    let again = dt::parse(&dt::serialize(&node));
    assert_eq!(again.name(), "chosen2");
    assert_eq!(again.get("boot-args"), Some(&b"-v\0"[..]));
}

#[test]
fn device_tree_show_formats_values() {
    assert_eq!(dt::show(&[]), "(empty)");
    assert_eq!(dt::show(b"abc\0"), "\"abc\"");
    assert_eq!(dt::show(&[0xde, 0xad, 0xbe]), "deadbe");
}
