#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod disk;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod logo;

#[cfg(windows)]
fn main() -> eframe::Result {
    app::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("anathema-setup writes USB sticks on Windows; on this system anathema-esp builds the EFI partition image");
    std::process::exit(1);
}
