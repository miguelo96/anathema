use std::{env, fs, path::PathBuf};

const IMAGE: &str = "ANATHEMA_EFI";

fn main() {
    println!("cargo:rerun-if-env-changed={IMAGE}");
    let image = env::var_os(IMAGE)
        .filter(|image| !image.is_empty())
        .unwrap_or_else(|| panic!("set {IMAGE} to the anathema.efi to embed"));
    let image = PathBuf::from(image);
    println!("cargo:rerun-if-changed={}", image.display());
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    fs::copy(&image, out.join("anathema.efi"))
        .unwrap_or_else(|error| panic!("cannot read {IMAGE} {}: {error}", image.display()));
}
