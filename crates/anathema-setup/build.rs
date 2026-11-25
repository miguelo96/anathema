use std::{env, fs, path::PathBuf};

const IMAGE: &str = "ANATHEMA_EFI";

/// A hash of the source that decides what the kernel files contain (the linker, the KDK
/// unpacker, the plist code and the link options), so a cache from an older setup is rebuilt.
fn linker_source_hash() -> String {
    use std::hash::{Hash, Hasher};
    fn collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display())).flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let here = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&here.join("../anathema-kc/src"), &mut files);
    collect(&here.join("../anathema-media/src/kdk"), &mut files);
    files.push(here.join("../anathema-media/src/plist.rs"));
    files.push(here.join("src/kernel_files.rs"));
    files.push(here.join("../../tools/kc-roots.txt"));
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        f.strip_prefix(&here).unwrap_or(f).hash(&mut h);
        fs::read(f).unwrap_or_else(|e| panic!("reading {}: {e}", f.display())).hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn main() {
    println!("cargo:rustc-env=ANATHEMA_LINKER_SOURCE={}", linker_source_hash());
    println!("cargo:rerun-if-env-changed={IMAGE}");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    match env::var_os(IMAGE).filter(|image| !image.is_empty()) {
        Some(image) => {
            let image = PathBuf::from(image);
            println!("cargo:rerun-if-changed={}", image.display());
            fs::copy(&image, out.join("anathema.efi"))
                .unwrap_or_else(|error| panic!("cannot read {IMAGE} {}: {error}", image.display()));
        }
        // the setup exe must carry the loader; elsewhere only the library and anathema-esp
        // (which takes --efi) are built, so an empty placeholder is enough
        None if windows => panic!("set {IMAGE} to the anathema-xboot .efi to embed"),
        None => fs::write(out.join("anathema.efi"), []).expect("write the placeholder loader"),
    }
}
