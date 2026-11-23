//! Host-side tests for the IPSW parsers in `anathema-media`.
//!
//! Every fixture is built by hand in this file, so the tests need no Apple software and no
//! network. Each parser gets a "good input" test and at least one "bad input" test: a corrupt
//! or hostile file must give an `Err`, never a panic or a huge allocation.
//!
//! Run with `cargo test -p anathema-media`.

use anathema_media::readat::{Cursor, ReadAt, Section};
use anathema_media::{hash, im4p, plist, zip};
use std::io::Read;

// ---------------------------------------------------------------- helpers

/// DER TLV with a short or two-byte long-form length.
fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    if body.len() < 0x80 {
        v.push(body.len() as u8);
    } else {
        assert!(body.len() <= 0xffff);
        v.push(0x82);
        v.extend_from_slice(&(body.len() as u16).to_be_bytes());
    }
    v.extend_from_slice(body);
    v
}

fn im4p_blob(kind: &str, desc: &str, payload: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut body = der(0x16, b"IM4P");
    body.extend(der(0x16, kind.as_bytes()));
    body.extend(der(0x16, desc.as_bytes()));
    body.extend(der(0x04, payload));
    body.extend_from_slice(extra);
    der(0x30, &body)
}

/// An LZSS container as found in older kernelcaches: "complzss" header, payload at 0x180.
fn complzss(uncompressed_len: u32, stream: &[u8]) -> Vec<u8> {
    let mut v = b"complzss".to_vec();
    v.extend_from_slice(&0u32.to_be_bytes()); // adler32, not checked by the reader
    v.extend_from_slice(&uncompressed_len.to_be_bytes());
    v.extend_from_slice(&(stream.len() as u32).to_be_bytes());
    v.resize(0x180, 0);
    v.extend_from_slice(stream);
    v
}

/// One Apple device tree property: 32-byte name, u32 length, value padded to 4 bytes.
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

fn dt_node(props: &[Vec<u8>], children: &[Vec<u8>]) -> Vec<u8> {
    let mut v = (props.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(&(children.len() as u32).to_le_bytes());
    for p in props {
        v.extend_from_slice(p);
    }
    for c in children {
        v.extend_from_slice(c);
    }
    v
}

/// A minimal zip with stored (method 0) or deflated (method 8) entries.
fn zip_archive(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, deflate) in files {
        let (method, body) = if *deflate {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            (8u16, e.finish().unwrap())
        } else {
            (0u16, data.to_vec())
        };
        let header = out.len() as u32;
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]); // version needed, flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0; 8]); // time, date, crc32 (not checked by the reader)
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);

        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0]); // made by, needed, flags
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]); // extra len, comment len, disk, int attr, ext attr
        central.extend_from_slice(&header.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

// ---------------------------------------------------------------- hash

#[test]
fn sha256_matches_known_vector() {
    assert_eq!(
        hash::sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha1_reader_matches_known_vector_and_reports_progress() {
    let mut seen = Vec::new();
    let h = hash::sha1_reader(&b"abc"[..], &mut |n| seen.push(n)).unwrap();
    assert_eq!(h, "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(seen, vec![3]);
}

#[test]
fn hex_is_lowercase_and_zero_padded() {
    assert_eq!(hash::hex(&[0x00, 0x0f, 0xab]), "000fab");
}

// ---------------------------------------------------------------- plist

const SAMPLE_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- a comment -->
    <key>ProductVersion</key>
    <string>27.2</string>
    <key>Build &amp; Train</key>
    <string>&lt;tag&gt; &#x41;&#66;</string>
    <key>Size</key>
    <integer>-42</integer>
    <key>Big</key>
    <integer>18446744073709551615</integer>
    <key>Ratio</key>
    <real>0.5</real>
    <key>Signed</key>
    <true/>
    <key>Blob</key>
    <data>
        aGVs
        bG8=
    </data>
    <key>Devices</key>
    <array>
        <dict>
            <key>Model</key>
            <string>VirtualMac2,1</string>
        </dict>
        <array/>
        <dict/>
    </array>
</dict>
</plist>"#;

#[test]
fn plist_parses_nested_values() {
    let v = plist::parse(SAMPLE_PLIST).unwrap();
    assert_eq!(v.get("ProductVersion").and_then(|v| v.as_str()), Some("27.2"));
    assert_eq!(v.get("Size").and_then(|v| v.as_int()), Some(-42));
    assert!(matches!(v.get("Ratio"), Some(plist::Value::Real(r)) if *r == 0.5));
    assert_eq!(v.get("Signed").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(v.get("Blob").and_then(|v| v.as_data()), Some(&b"hello"[..]));
    let devices = v.get("Devices").and_then(|v| v.as_array()).unwrap();
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].get("Model").and_then(|v| v.as_str()), Some("VirtualMac2,1"));
    assert_eq!(devices[1].as_array().map(|a| a.len()), Some(0));
    assert_eq!(devices[2].as_dict().map(|d| d.len()), Some(0));
}

#[test]
fn plist_unescapes_entities_in_keys_and_strings() {
    let v = plist::parse(SAMPLE_PLIST).unwrap();
    assert_eq!(v.get("Build & Train").and_then(|v| v.as_str()), Some("<tag> AB"));
}

#[test]
fn plist_integer_above_i64_max_wraps() {
    // Documents current behaviour: values above i64::MAX are stored bit-for-bit as i64.
    let v = plist::parse(SAMPLE_PLIST).unwrap();
    assert_eq!(v.get("Big").and_then(|v| v.as_int()), Some(-1));
}

#[test]
fn plist_path_walks_dicts() {
    let v = plist::parse("<plist><dict><key>a</key><dict><key>b</key><string>c</string></dict></dict></plist>")
        .unwrap();
    assert_eq!(v.path(&["a", "b"]).and_then(|v| v.as_str()), Some("c"));
    assert!(v.path(&["a", "missing"]).is_none());
}

#[test]
fn plist_rejects_bad_input() {
    for bad in [
        "bplist00\x00\x01",                          // binary plist
        "",                                          // empty
        "<plist><dict><key>a</key>",                 // unterminated dict
        "<plist><dict></array></plist>",             // mismatched close
        "<plist><dict><string>x</string></dict></plist>", // value without key
        "<plist><array><key>a</key></array></plist>",     // key outside dict
        "<plist><string>never closed</plist>",       // missing </string>
        "<plist><integer>12x</integer></plist>",     // not a number
        "<plist><data>!!!</data></plist>",           // not base64
        "<plist><bogus/></plist>",                   // unknown tag
        "<plist><dict",                              // unterminated tag
    ] {
        assert!(plist::parse(bad).is_err(), "expected an error for {bad:?}");
    }
}

// ---------------------------------------------------------------- im4p / der

#[test]
fn der_tlv_reads_short_and_long_lengths() {
    let short = der(0x04, b"abc");
    let t = im4p::tlv(&short).unwrap();
    assert_eq!((t.tag, t.body, t.rest), (0x04, &b"abc"[..], &b""[..]));

    let body = vec![7u8; 300];
    let mut long = der(0x04, &body);
    long.push(0xee);
    let t = im4p::tlv(&long).unwrap();
    assert_eq!(t.body.len(), 300);
    assert_eq!(t.rest, &[0xee]);
}

#[test]
fn der_int_is_big_endian() {
    assert_eq!(im4p::der_int(&[0x01, 0x00]), 256);
    assert_eq!(im4p::der_int(&[]), 0);
}

#[test]
fn der_tlv_rejects_truncated_and_oversized_lengths() {
    assert!(im4p::tlv(&[]).is_err());
    assert!(im4p::tlv(&[0x04]).is_err());
    assert!(im4p::tlv(&[0x04, 0x05, 1, 2]).is_err()); // body shorter than length
    assert!(im4p::tlv(&[0x04, 0x80]).is_err()); // indefinite length
    assert!(im4p::tlv(&[0x04, 0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err()); // 9 length bytes
    // A length of 2^64-1 must not overflow `header + length` and panic.
    assert!(im4p::tlv(&[0x04, 0x88, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
}

#[test]
fn im4p_parses_fields_and_compression_info() {
    let compression = der(0x30, &[der(0x02, &[1]), der(0x02, &[0x10, 0x00])].concat());
    let blob = im4p_blob("krnl", "KernelCacheBuilder", b"payload", &compression);
    let im = im4p::parse(&blob).unwrap();
    assert_eq!(im.kind, "krnl");
    assert_eq!(im.desc, "KernelCacheBuilder");
    assert_eq!(im.payload, b"payload");
    assert_eq!(im.compression, Some((1, 4096)));
    assert!(im.kbag.is_none());
    assert!(im4p::is_im4p(&blob));
}

#[test]
fn im4p_decode_refuses_encrypted_payloads() {
    let blob = im4p_blob("rdsk", "ramdisk", b"secret", &der(0x04, b"keybag"));
    let err = im4p::decode(&blob).err().expect("encrypted payload must not decode");
    assert!(err.to_string().contains("encrypted"), "{err}");
}

#[test]
fn im4p_decode_passes_uncompressed_payload_through() {
    let blob = im4p_blob("dtre", "DeviceTree", b"\x01\x02\x03", &[]);
    let (im, out) = im4p::decode(&blob).unwrap();
    assert_eq!(im.kind, "dtre");
    assert_eq!(out, [1, 2, 3]);
}

#[test]
fn im4p_rejects_wrong_magic_and_payload_type() {
    let mut wrong_magic = der(0x16, b"IMG4");
    wrong_magic.extend(der(0x16, b"krnl"));
    assert!(im4p::parse(&der(0x30, &wrong_magic)).is_err());
    assert!(im4p::parse(&der(0x31, b"")).is_err());

    let mut body = der(0x16, b"IM4P");
    body.extend(der(0x16, b"krnl"));
    body.extend(der(0x16, b"d"));
    body.extend(der(0x16, b"payload in an IA5String instead of an OCTET STRING"));
    assert!(im4p::parse(&der(0x30, &body)).is_err());
    assert!(!im4p::is_im4p(b"not der at all"));
}

#[test]
fn lzss_decodes_literals_and_back_references() {
    // flags 0b0000_0111: three literals, then one back reference.
    // The reference points at ring position 0xFEE (where the literals were written) with
    // length 6, so it repeats "abc" twice.
    let stream = [0x07, b'a', b'b', b'c', 0xEE, 0xF3];
    let out = im4p::decompress(&complzss(9, &stream), None).unwrap();
    assert_eq!(out, b"abcabcabc");
}

#[test]
fn lzss_rejects_truncated_input_without_panicking() {
    // Header cut off before the size fields.
    assert!(im4p::decompress(b"complzss\x00\x00\x00\x00", None).is_err());
    // Header says 5 bytes, stream is one flag byte that promises literals that are not there.
    assert!(im4p::decompress(&complzss(5, &[0xff]), None).is_err());
    // Compressed size larger than the file.
    let mut lying = complzss(4, b"\xffabcd");
    lying[16..20].copy_from_slice(&1000u32.to_be_bytes());
    assert!(im4p::decompress(&lying, None).is_err());
}

#[test]
fn decompress_leaves_unknown_data_alone() {
    assert_eq!(im4p::decompress(b"plain bytes", None).unwrap(), b"plain bytes");
}

// ---------------------------------------------------------------- device tree

#[test]
fn devicetree_parses_names_props_and_children() {
    let cpu = dt_node(&[dt_prop("name", b"cpu0\0")], &[]);
    let root = dt_node(&[dt_prop("name", b"device-tree\0"), dt_prop("compatible", b"abc")], &[cpu]);
    let n = im4p::parse_devicetree(&root).unwrap();
    assert_eq!(n.name(), "device-tree");
    assert_eq!(n.prop("compatible"), Some(&b"abc"[..]));
    assert_eq!(n.children.len(), 1);
    assert_eq!(n.children[0].name(), "cpu0");
    assert!(n.prop("missing").is_none());
}

#[test]
fn devicetree_masks_the_placeholder_bit_in_lengths() {
    // Apple sets bit 31 on "placeholder" property lengths; the reader masks it off.
    let mut p = dt_prop("x", b"abcd");
    p[32..36].copy_from_slice(&(0x8000_0004u32).to_le_bytes());
    let n = im4p::parse_devicetree(&dt_node(&[p], &[])).unwrap();
    assert_eq!(n.prop("x"), Some(&b"abcd"[..]));
}

#[test]
fn devicetree_rejects_truncated_and_hostile_input() {
    assert!(im4p::parse_devicetree(&[]).is_err());
    assert!(im4p::parse_devicetree(&[1, 0, 0, 0, 0, 0, 0, 0]).is_err()); // 1 prop, no bytes
    let mut p = dt_prop("x", b"ab");
    p[32..36].copy_from_slice(&1000u32.to_le_bytes()); // length past the end
    assert!(im4p::parse_devicetree(&dt_node(&[p], &[])).is_err());
    // Claims 4 billion properties and 4 billion children in an 8-byte file. Must fail fast,
    // not try to reserve hundreds of gigabytes up front.
    assert!(im4p::parse_devicetree(&[0xff; 8]).is_err());
}

// ---------------------------------------------------------------- zip

#[test]
fn zip_reads_stored_and_deflated_entries() {
    let big = b"anathema ".repeat(1000);
    let archive = zip_archive(&[("BuildManifest.plist", b"<plist/>", false), ("kernelcache", &big, true)]);
    let z = zip::Zip::open(&archive).unwrap();
    assert_eq!(z.entries.len(), 2);
    assert_eq!(z.read("BuildManifest.plist").unwrap(), b"<plist/>");
    let e = z.find("kernelcache").unwrap().clone();
    assert_eq!((e.method, e.size), (8, big.len() as u64));
    assert!(e.csize < e.size);
    assert_eq!(z.read("kernelcache").unwrap(), big);

    let mut out = Vec::new();
    let mut last = 0;
    let n = z.copy_to(&e, &mut out, &mut |t| last = t).unwrap();
    assert_eq!((n, last), (big.len() as u64, big.len() as u64));
    assert_eq!(out, big);
}

#[test]
fn zip_reports_missing_entries() {
    let archive = zip_archive(&[("a", b"1", false)]);
    let z = zip::Zip::open(&archive).unwrap();
    assert!(z.find("b").is_none());
    assert!(z.read("b").is_err());
}

#[test]
fn zip_rejects_non_zip_input() {
    assert!(zip::Zip::open(&Vec::new()).is_err());
    assert!(zip::Zip::open(&b"PK but not really a zip file at all".to_vec()).is_err());
}

#[test]
fn zip_rejects_truncated_central_directory_without_panicking() {
    let mut archive = zip_archive(&[("a-long-file-name", b"1", false)]);
    // Point the name length of the only central directory entry far past the end.
    let cd = archive.windows(4).position(|w| w == 0x02014b50u32.to_le_bytes()).unwrap();
    archive[cd + 28..cd + 30].copy_from_slice(&0xffffu16.to_le_bytes());
    let result = std::panic::catch_unwind(|| zip::Zip::open(&archive).map(|z| z.entries.len()));
    assert!(matches!(result, Ok(Err(_))), "a corrupt central directory must give an Err, not a panic");
}

// ---------------------------------------------------------------- readat

#[test]
fn section_and_cursor_stay_inside_their_window() {
    let data: Vec<u8> = (0u8..100).collect();
    let s = Section { inner: &data, start: 10, size: 5 };
    assert_eq!(s.len(), 5);
    assert_eq!(s.read_vec(0, 5).unwrap(), [10, 11, 12, 13, 14]);
    let mut buf = [0u8; 8];
    assert_eq!(s.read_at(3, &mut buf).unwrap(), 2);
    assert_eq!(s.read_at(5, &mut buf).unwrap(), 0);
    assert!(s.read_vec(3, 4).is_err()); // short read is an error, not silent zeros

    let mut c = Cursor::new(&data, 98, 200);
    let mut all = Vec::new();
    c.read_to_end(&mut all).unwrap();
    assert_eq!(all, [98, 99]);
}
