use crate::plist::{self, Value};
use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub device_class: String,
    pub variant: String,
    pub chip: String,
    pub board: String,
    pub components: Vec<Component>,
}

impl Identity {
    pub fn path(&self, name: &str) -> Option<&str> {
        self.components.iter().find(|c| c.name == name).map(|c| c.path.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub build: String,
    pub version: String,
    pub products: Vec<String>,
    pub identities: Vec<Identity>,
}

pub fn device_class(product: &str) -> Option<&'static str> {
    Some(match product {
        "Mac14,3" => "j473ap",
        "Mac14,12" => "j474sap",
        "Mac14,2" => "j413ap",
        "Mac14,7" => "j493ap",
        "Mac14,15" => "j515ap",
        "Macmini9,1" => "j274ap",
        "Mac15,12" => "j613ap",
        _ => return None,
    })
}

pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    let root = plist::parse(std::str::from_utf8(bytes).context("manifest is not utf-8")?)?;
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_string();
    let products = root
        .get("SupportedProductTypes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut identities = Vec::new();
    for id in root.get("BuildIdentities").and_then(Value::as_array).context("no BuildIdentities")? {
        let mut components = Vec::new();
        if let Some(m) = id.get("Manifest").and_then(Value::as_dict) {
            for (name, c) in m {
                if let Some(p) = c.path(&["Info", "Path"]).and_then(Value::as_str) {
                    components.push(Component { name: name.clone(), path: p.to_string() });
                }
            }
        }
        identities.push(Identity {
            device_class: text(id.path(&["Info", "DeviceClass"])),
            variant: text(id.path(&["Info", "Variant"])),
            chip: text(id.get("ApChipID")),
            board: text(id.get("ApBoardID")),
            components,
        });
    }
    Ok(Manifest {
        build: text(root.get("ProductBuildVersion")),
        version: text(root.get("ProductVersion")),
        products,
        identities,
    })
}

impl Manifest {
    pub fn select(&self, class: &str, variant_hint: Option<&str>) -> Result<&Identity> {
        let mut matches: Vec<&Identity> = self
            .identities
            .iter()
            .filter(|i| i.device_class.eq_ignore_ascii_case(class))
            .collect();
        if matches.is_empty() {
            let mut classes: Vec<_> = self.identities.iter().map(|i| i.device_class.clone()).collect();
            classes.sort();
            classes.dedup();
            bail!("no build identity for {class}; available: {}", classes.join(" "));
        }
        if let Some(h) = variant_hint {
            if let Some(i) = matches.iter().find(|i| i.variant.contains(h)) {
                return Ok(i);
            }
        }
        matches.sort_by_key(|i| (!i.variant.contains("Customer"), i.variant.contains("Research"), i.variant.len()));
        Ok(matches[0])
    }
}
