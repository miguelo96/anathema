use alloc::format;
use alloc::string::String;
use uefi::boot;
use uefi::cstr16;
use uefi::fs::{FileSystem, PathBuf};

pub fn save(text: &str) -> String {
    let proto = match boot::get_image_file_system(boot::image_handle()) {
        Ok(proto) => proto,
        Err(error) => return format!("log not saved: {:?}", error.status()),
    };
    let mut fs = FileSystem::new(proto);
    let path = PathBuf::from(cstr16!("\\anathema.log"));
    match fs.write(&path, text.as_bytes()) {
        Ok(()) => String::from("saved to anathema.log on the boot volume"),
        Err(error) => format!("log not saved: {}", error),
    }
}
