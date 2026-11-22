use alloc::string::String;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
static mut VARS: Vec<(String, String)> = Vec::new();
static mut SINK: Option<fn(&str)> = None;

pub fn set_sink(f: fn(&str)) {
    unsafe {
        SINK = Some(f);
    }
}

#[cfg(not(feature = "std"))]
pub fn set_var(name: &str, val: &str) {
    unsafe {
        let v = &mut *core::ptr::addr_of_mut!(VARS);
        v.retain(|x| x.0 != name);
        v.push((String::from(name), String::from(val)));
    }
}

#[cfg(feature = "std")]
pub fn var(name: &str) -> Result<String, ()> {
    std::env::var(name).map_err(|_| ())
}

#[cfg(not(feature = "std"))]
pub fn var(name: &str) -> Result<String, ()> {
    unsafe {
        let v = &*core::ptr::addr_of!(VARS);
        v.iter().find(|x| x.0 == name).map(|x| x.1.clone()).ok_or(())
    }
}

#[cfg(feature = "std")]
pub fn nanos() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

#[cfg(not(feature = "std"))]
pub fn nanos() -> u64 {
    0
}

pub fn log(s: &str) {
    unsafe {
        match SINK {
            Some(f) => f(s),
            None => {
                #[cfg(feature = "std")]
                {
                    print!("{}", s);
                }
            }
        }
    }
}
