use std::{env, fs, path::PathBuf};

const IMAGE: &str = "ANATHEMA_EFI";

fn main() {
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
