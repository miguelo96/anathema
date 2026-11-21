use crate::http;
use crate::plist::{self, Value};
use anyhow::{Context, Result};

pub const FEED_URL: &str = "https://mesu.apple.com/assets/macos/com_apple_macOSIPSW/com_apple_macOSIPSW.xml";
pub const DEFAULT_PRODUCT: &str = "Mac14,3";

#[derive(Debug, Clone)]
pub struct Candidate {
    pub product: String,
    pub build: String,
    pub version: String,
    pub url: String,
    pub sha1: String,
}

impl Candidate {
    pub fn file_name(&self) -> &str {
        self.url.rsplit('/').next().unwrap_or("restore.ipsw")
    }
}

pub fn version_key(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn build_key(b: &str) -> (u64, String, u64) {
    let digits: String = b.chars().take_while(|c| c.is_ascii_digit()).collect();
    let rest: String = b.chars().skip(digits.len()).collect();
    let letter: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let num: String = rest.chars().skip(letter.len()).take_while(|c| c.is_ascii_digit()).collect();
    (digits.parse().unwrap_or(0), letter, num.parse().unwrap_or(0))
}

fn collect(v: &Value, product: Option<&str>, out: &mut Vec<Candidate>) {
    match v {
        Value::Dict(items) => {
            if let (Some(p), Some(r)) = (product, v.get("Restore")) {
                if let (Some(url), Some(build)) = (
                    r.get("FirmwareURL").and_then(Value::as_str),
                    r.get("BuildVersion").and_then(Value::as_str),
                ) {
                    out.push(Candidate {
                        product: p.to_string(),
                        build: build.to_string(),
                        version: r.get("ProductVersion").and_then(Value::as_str).unwrap_or("").to_string(),
                        url: url.to_string(),
                        sha1: r.get("FirmwareSHA1").and_then(Value::as_str).unwrap_or("").to_string(),
                    });
                }
            }
            for (k, child) in items {
                if k == "MobileDeviceSoftwareVersions" {
                    if let Value::Dict(products) = child {
                        for (name, pv) in products {
                            collect(pv, Some(name), out);
                        }
                    }
                } else {
                    collect(child, product, out);
                }
            }
        }
        Value::Array(a) => {
            for c in a {
                collect(c, product, out);
            }
        }
        _ => {}
    }
}

pub fn parse_feed(xml: &str) -> Result<Vec<Candidate>> {
    let root = plist::parse(xml)?;
    let mut out = Vec::new();
    collect(&root, None, &mut out);
    out.sort_by(|a, b| {
        a.product
            .cmp(&b.product)
            .then(version_key(&b.version).cmp(&version_key(&a.version)))
            .then(build_key(&b.build).cmp(&build_key(&a.build)))
    });
    out.dedup_by(|a, b| a.product == b.product && a.build == b.build && a.url == b.url);
    Ok(out)
}

pub fn fetch_feed() -> Result<Vec<Candidate>> {
    let client = http::client()?;
    let body = http::get_bytes(&client, FEED_URL)?;
    parse_feed(std::str::from_utf8(&body).context("feed is not utf-8")?)
}

pub fn for_product<'a>(all: &'a [Candidate], product: &str) -> Vec<&'a Candidate> {
    all.iter().filter(|c| c.product == product).collect()
}

pub fn pick<'a>(all: &'a [Candidate], product: &str, build: Option<&str>) -> Option<&'a Candidate> {
    let list = for_product(all, product);
    match build {
        Some(b) => list.into_iter().find(|c| c.build == b),
        None => list.into_iter().next(),
    }
}
