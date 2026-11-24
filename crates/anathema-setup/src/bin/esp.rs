//! anathema-esp: writes the stick's EFI partition as an image file, for QEMU and for testing
//! on macOS or Linux.
//!
//! usage: anathema-esp --efi LOADER.efi --kernel xkernel --xkc xkc.bin --out esp.img [--size-mib 200]
//!        anathema-esp --efi LOADER.efi --cache DIR --out esp.img   (downloads and builds the kernel
//!        files in DIR exactly as setup does)

use anathema_setup::fat32::{self, FileSpec};
use anathema_setup::kernel_files::{self, KernelFiles};
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut efi, mut kernel, mut xkc, mut out, mut size_mib, mut cache) = (None, None, None, None, 200u64, None);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--efi" => efi = Some(PathBuf::from(val()?)),
            "--kernel" => kernel = Some(PathBuf::from(val()?)),
            "--xkc" => xkc = Some(PathBuf::from(val()?)),
            "--out" => out = Some(PathBuf::from(val()?)),
            "--cache" => cache = Some(PathBuf::from(val()?)),
            "--size-mib" => size_mib = val()?.parse().map_err(|e| format!("--size-mib: {e}"))?,
            _ => return Err(format!("unknown argument {a}; usage: anathema-esp --efi F --kernel F --xkc F --out esp.img [--size-mib 200]")),
        }
    }
    let read = |p: Option<PathBuf>, what: &str| -> Result<Vec<u8>, String> {
        let p = p.ok_or(format!("--{what} is required"))?;
        std::fs::read(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))
    };
    let efi = read(efi, "efi")?;
    let out = out.ok_or("--out is required")?;
    let k = match cache {
        Some(cache) => {
            let mut last = std::time::Instant::now();
            kernel_files::prepare(
                &cache,
                u64::MAX,
                &mut |p, stage| {
                    if last.elapsed().as_secs() >= 2 {
                        eprintln!("{:5.1}% {stage}", p * 100.0);
                        last = std::time::Instant::now();
                    }
                },
                &mut |line| eprintln!("{line}"),
            )?
        }
        None => KernelFiles { xkernel: read(kernel, "kernel")?, xkc: read(xkc, "xkc")? },
    };
    let files: Vec<FileSpec> = kernel_files::esp_files(&efi, &k);
    let sectors = (size_mib << 20) / fat32::SECTOR;
    let img = fat32::image(&fat32::build(sectors, 0, "ANATHEMA", &files)?, sectors);
    std::fs::write(&out, img).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    println!("{} ({size_mib} MiB)", out.display());
    Ok(())
}
