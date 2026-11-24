//! Expected strings were produced by running `xml()` from tools/kcbuild.py on `plistlib.loads`
//! of the same input.

use anathema_kc::plistxml::{document, normalize, py_float_repr, write};
use anathema_media::plist::parse;

fn out(input: &str) -> String {
    write(&normalize(parse(input).unwrap()), 0)
}

const MIXED_IN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>com.example.a&amp;b&lt;c&gt;\"q\"</string>\n\t<key>Neg</key>\n\t<integer>-1</integer>\n\t<key>Hex</key>\n\t<integer>0xffffff8002000000</integer>\n\t<key>Flag</key>\n\t<true/>\n\t<key>Off</key>\n\t<false/>\n\t<key>Blob</key>\n\t<data>\n\tAAECAwQF\n\tBgc=\n\t</data>\n\t<key>Libs</key>\n\t<dict>\n\t\t<key>com.apple.kpi.libkern</key>\n\t\t<string>8.0.0</string>\n\t</dict>\n\t<key>List</key>\n\t<array>\n\t\t<string>x</string>\n\t\t<integer>3</integer>\n\t\t<real>0.5</real>\n\t</array>\n</dict>\n</plist>\n";
const MIXED_OUT: &str = "<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>com.example.a&amp;b&lt;c&gt;\"q\"</string>\n\t<key>Neg</key>\n\t<integer size=\"64\">18446744073709551615</integer>\n\t<key>Hex</key>\n\t<integer size=\"64\">18446743523987292160</integer>\n\t<key>Flag</key>\n\t<true/>\n\t<key>Off</key>\n\t<false/>\n\t<key>Blob</key>\n\t<data>AAECAwQFBgc=</data>\n\t<key>Libs</key>\n\t<dict>\n\t\t<key>com.apple.kpi.libkern</key>\n\t\t<string>8.0.0</string>\n\t</dict>\n\t<key>List</key>\n\t<array>\n\t\t<string>x</string>\n\t\t<integer size=\"64\">3</integer>\n\t\t<real>0.5</real>\n\t</array>\n</dict>";
const EMPTY_IN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict><key>A</key><array/><key>D</key><dict/><key>E</key><array></array><key>S</key><string></string><key>T</key><string/></dict>\n</plist>\n";
const EMPTY_OUT: &str = "<dict>\n\t<key>A</key>\n\t<array>\n\n\t</array>\n\t<key>D</key>\n\t<dict>\n\t</dict>\n\t<key>E</key>\n\t<array>\n\n\t</array>\n\t<key>S</key>\n\t<string></string>\n\t<key>T</key>\n\t<string></string>\n</dict>";
const DUP_IN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict><key>K</key><string>1</string><key>Z</key><string>z</string><key>K</key><string>2</string></dict>\n</plist>\n";
const DUP_OUT: &str = "<dict>\n\t<key>K</key>\n\t<string>2</string>\n\t<key>Z</key>\n\t<string>z</string>\n</dict>";
const CRLF_IN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict><key>K\r\nX</key><string>a\r\nb\rc</string></dict>\n</plist>\n";
const CRLF_OUT: &str = "<dict>\n\t<key>K\nX</key>\n\t<string>a\nb\nc</string>\n</dict>";
const DATE_IN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict><key>When</key><date>2026-10-14T23:40:09Z</date></dict>\n</plist>\n";
const DATE_OUT: &str = "<dict>\n\t<key>When</key>\n\t<string>2026-10-14T23:40:09</string>\n</dict>";
const DOC_OUT: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>_PrelinkInfoDictionary</key>\n\t<array>\n\t\t<dict>\n\t\t\t<key>K</key>\n\t\t\t<string>2</string>\n\t\t\t<key>Z</key>\n\t\t\t<string>z</string>\n\t\t</dict>\n\t</array>\n</dict>\n</plist>\n\0";

#[test]
fn plistxml_matches_python_for_mixed_dict() {
    assert_eq!(out(MIXED_IN), MIXED_OUT);
}

#[test]
fn plistxml_empty_containers() {
    assert_eq!(out(EMPTY_IN), EMPTY_OUT);
}

#[test]
fn plistxml_duplicate_keys_last_wins_first_position() {
    assert_eq!(out(DUP_IN), DUP_OUT);
}

#[test]
fn plistxml_crlf_normalized() {
    assert_eq!(out(CRLF_IN), CRLF_OUT);
}

#[test]
fn plistxml_date_isoformat() {
    assert_eq!(out(DATE_IN), DATE_OUT);
}

#[test]
fn plistxml_document_wraps_infos() {
    let info = normalize(parse(DUP_IN).unwrap());
    assert_eq!(document(&[info]), DOC_OUT.as_bytes());
}

#[test]
fn py_float_repr_cases() {
    let cases: [(f64, &str); 13] = [
        (1.0, "1.0"),
        (0.5, "0.5"),
        (1e-05, "1e-05"),
        (0.0001, "0.0001"),
        (1e16, "1e+16"),
        (123456789012345.6, "123456789012345.6"),
        (-0.0, "-0.0"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (12345678901234567.0, "1.2345678901234568e+16"),
        (1.5e300, "1.5e+300"),
        (100.0, "100.0"),
        (0.1, "0.1"),
    ];
    for (f, want) in cases {
        assert_eq!(py_float_repr(f), want, "repr of {f:e}");
    }
    assert_eq!(py_float_repr(f64::NAN), "nan");
}
