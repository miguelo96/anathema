//! Finds kext bundles under the `--ext` folders, like `find_bundles` in kcbuild.py.

use crate::plistxml::normalize;
use anathema_media::plist::{self, Value};
use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Bundle {
    /// The `.kext` folder.
    pub path: PathBuf,
    /// Its Info.plist.
    pub plist: PathBuf,
    /// The parsed, normalized Info.plist (a dict).
    pub info: Value,
    /// The `--ext` folder it was found under (for an override: the folder holding the kext).
    pub root: PathBuf,
}

/// Reads an Info.plist the way kcbuild.py's `plistlib.load` sees it.
pub fn read_info(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("{} is not utf-8", path.display()))?;
    Ok(normalize(plist::parse(&text).with_context(|| format!("parsing {}", path.display()))?))
}

/// Every bundle under `roots`, first bundle id wins, in a depth-first walk with folder names
/// sorted by their bytes. (kcbuild.py walks in file system order; see the design doc.)
pub fn find_bundles(roots: &[PathBuf]) -> Vec<(String, Bundle)> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        walk(root, root, &mut found, &mut seen);
    }
    found
}

fn walk(dir: &Path, root: &Path, found: &mut Vec<(String, Bundle)>, seen: &mut HashSet<String>) {
    if dir.as_os_str().as_encoded_bytes().ends_with(b".kext") {
        let plist = [dir.join("Contents").join("Info.plist"), dir.join("Info.plist")].into_iter().find(|p| p.exists());
        if let Some(plist) = plist {
            if let Ok(info) = read_info(&plist) {
                let id = info.get("CFBundleIdentifier").and_then(Value::as_str).unwrap_or("").to_string();
                if !id.is_empty() && seen.insert(id.clone()) {
                    found.push((id, Bundle { path: dir.to_path_buf(), plist, info, root: root.to_path_buf() }));
                }
            }
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    subdirs.sort_by(|a, b| a.file_name().unwrap().as_encoded_bytes().cmp(b.file_name().unwrap().as_encoded_bytes()));
    for sub in subdirs {
        walk(&sub, root, found, seen);
    }
}

/// `os.path.relpath(path, base)` with `/` separators, for paths under `base`.
pub fn rel(path: &Path, base: &Path) -> String {
    let p = path.strip_prefix(base).unwrap_or(path);
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}
