//! Writes plists exactly the way `xml()` in `tools/kcbuild.py` does, so the prelink info the
//! kernel parses is byte-identical to the Python linker's.

use anathema_media::plist::Value;
use base64::Engine;

const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n";

/// Makes a parsed plist look like `plistlib.loads` would leave it: duplicate dict keys keep their
/// first position with the last value, line ends are `\n` as an XML parser reports them, and
/// dates become the text `datetime.isoformat()` prints.
pub fn normalize(v: Value) -> Value {
    match v {
        Value::Dict(items) => {
            let mut out: Vec<(String, Value)> = Vec::with_capacity(items.len());
            for (k, v) in items {
                let k = newlines(k);
                let v = normalize(v);
                match out.iter_mut().find(|(have, _)| *have == k) {
                    Some(slot) => slot.1 = v,
                    None => out.push((k, v)),
                }
            }
            Value::Dict(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(normalize).collect()),
        Value::Str(s) => Value::Str(newlines(s)),
        Value::Date(d) => Value::Date(isoformat(d.trim())),
        other => other,
    }
}

fn newlines(s: String) -> String {
    if s.contains('\r') {
        s.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        s
    }
}

/// plistlib reads `2026-10-14T23:40:09Z` as a naive datetime; isoformat drops the zone.
fn isoformat(d: &str) -> String {
    d.strip_suffix('Z').unwrap_or(d).to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Python's `repr()` of a float.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // shortest round-trip digits and the decimal exponent, e.g. "1.2345e16"
    let sci = format!("{f:e}");
    let (mant, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let exp: i32 = exp.parse().expect("exponent is an integer");
    let (sign, mant) = match mant.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mant),
    };
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    if (-4..16).contains(&exp) {
        let point = exp + 1;
        let body = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{body}")
    } else {
        let frac = if digits.len() > 1 { format!(".{}", &digits[1..]) } else { String::new() };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{}{frac}e{esign}{:02}", &digits[..1], exp.abs())
    }
}

/// `xml(v, ind)` from kcbuild.py.
pub fn write(v: &Value, ind: usize) -> String {
    let pad = "\t".repeat(ind);
    match v {
        Value::Bool(true) => format!("{pad}<true/>"),
        Value::Bool(false) => format!("{pad}<false/>"),
        Value::Int(i) => format!("{pad}<integer size=\"64\">{}</integer>", *i as u64),
        Value::Real(f) => format!("{pad}<real>{}</real>", py_float_repr(*f)),
        Value::Str(s) => format!("{pad}<string>{}</string>", esc(s)),
        Value::Date(d) => format!("{pad}<string>{}</string>", esc(d)),
        Value::Data(d) => format!("{pad}<data>{}</data>", base64::engine::general_purpose::STANDARD.encode(d)),
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(|x| write(x, ind + 1)).collect();
            format!("{pad}<array>\n{}\n{pad}</array>", inner.join("\n"))
        }
        Value::Dict(items) => {
            let mut out = vec![format!("{pad}<dict>")];
            for (k, x) in items {
                out.push(format!("{pad}\t<key>{}</key>", esc(k)));
                out.push(write(x, ind + 1));
            }
            out.push(format!("{pad}</dict>"));
            out.join("\n")
        }
    }
}

/// The whole prelink info plist, NUL terminated, as kcbuild.py writes it.
pub fn document(infos: &[Value]) -> Vec<u8> {
    let root = Value::Dict(vec![("_PrelinkInfoDictionary".into(), Value::Array(infos.to_vec()))]);
    let mut s = String::from(HEADER);
    s.push_str(&write(&root, 0));
    s.push_str("\n</plist>\n\0");
    s.into_bytes()
}
