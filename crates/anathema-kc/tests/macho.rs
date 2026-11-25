mod support;

use anathema_kc::macho::{kernel_symbols, x86_64_slice, Macho};
use support::*;

fn sample() -> KextSpec {
    KextSpec {
        name: "Sample",
        text: vec![0x90; 32],
        data: vec![0; 0x200],
        exports: vec![("_foo", 0x1000), ("_bar", 0x4010)],
        imports: vec!["_ext"],
        kmod_info_at: Some(0x4100),
        uuid: Some(*b"0123456789abcdef"),
        ..Default::default()
    }
}

#[test]
fn slice_picks_x86_64_from_fat() {
    let x86 = build_kext(&sample());
    let arm = vec![0xAAu8; 64];
    let f = fat(&[(CPU_ARM64, &arm), (CPU_X86_64, &x86)]);
    assert_eq!(x86_64_slice(&f).as_deref(), Some(&x86[..]));
}

#[test]
fn slice_accepts_thin_x86_64() {
    let x86 = build_kext(&sample());
    assert_eq!(x86_64_slice(&x86).as_deref(), Some(&x86[..]));
}

#[test]
fn slice_rejects_arm_only() {
    let arm = vec![0xAAu8; 64];
    assert_eq!(x86_64_slice(&fat(&[(CPU_ARM64, &arm)])), None);
    assert_eq!(x86_64_slice(b"not a mach-o at all"), None);
}

#[test]
fn parse_reads_segments_sections_symbols() {
    let m = Macho::parse(build_kext(&sample())).unwrap();
    let names: Vec<&str> = m.segs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["__TEXT", "__DATA", "__LINKEDIT"]);
    assert_eq!((m.segs[1].vm, m.segs[1].vs, m.segs[1].fo, m.segs[1].fs), (0x4000, 0x4000, 0x4000, 0x4000));
    assert_eq!(m.sects.len(), 2);
    assert_eq!((m.sects[1].seg.as_str(), m.sects[1].name.as_str(), m.sects[1].addr), ("__DATA", "__data", 0x4000));
    assert_eq!(m.ncmds, 6);
    let s = m.sym_at(3).unwrap();
    assert_eq!((s.name.as_str(), s.n_type, s.value), ("_ext", 0x01, 0));
}

#[test]
fn exports_in_table_order() {
    let m = Macho::parse(build_kext(&sample())).unwrap();
    let e = m.exports().unwrap();
    assert_eq!(e, vec![("_foo".to_string(), 0x1000), ("_bar".to_string(), 0x4010), ("_kmod_info".to_string(), 0x4100)]);
}

#[test]
fn find_sym_finds_kmod_info() {
    let m = Macho::parse(build_kext(&sample())).unwrap();
    assert_eq!(m.find_sym("_kmod_info").unwrap(), Some(0x4100));
    assert_eq!(m.find_sym("_ext").unwrap(), None, "undefined symbols are not definitions");
    assert_eq!(m.find_sym("_missing").unwrap(), None);
}

#[test]
fn kernel_symbols_prefers_external_over_local() {
    let spec = KextSpec {
        exports: vec![("_dup", 2), ("_only_ext", 5)],
        locals: vec![("_dup", 1), ("_only_local", 7), ("_only_local", 9)],
        ..sample()
    };
    let syms = kernel_symbols(&Macho::parse(build_kext(&spec)).unwrap()).unwrap();
    assert_eq!(syms["_dup"], 2);
    assert_eq!(syms["_only_ext"], 5);
    assert_eq!(syms["_only_local"], 7, "first local wins");
    assert!(!syms.contains_key("_ext"));
}

#[test]
fn uuid_reads_lc_uuid() {
    let m = Macho::parse(build_kext(&sample())).unwrap();
    assert_eq!(m.uuid(), Some(*b"0123456789abcdef"));
    let m2 = Macho::parse(build_kext(&KextSpec { uuid: None, ..sample() })).unwrap();
    assert_eq!(m2.uuid(), None);
}

#[test]
fn parse_rejects_truncated_input() {
    let mut d = build_kext(&sample());
    d.truncate(40);
    assert!(Macho::parse(d).is_err());
}
