use alloc::vec::Vec;
use uefi::runtime::{self, VariableAttributes, VariableVendor};
use uefi::{cstr16, guid};

const HEAD: usize = 3 * 1024;
const TAIL: usize = 9 * 1024;
const MARK: &[u8] = b"\r\n[...]\r\n";

static mut STAGE: [u8; HEAD + TAIL + 16] = [0; HEAD + TAIL + 16];

fn vendor() -> VariableVendor {
    VariableVendor(guid!("5b0c9cb0-8f3a-4f6e-9d17-a7e1c0d7a001"))
}

fn attributes() -> VariableAttributes {
    VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS
}

pub fn save(data: &[u8]) -> bool {
    let stage = unsafe { &mut *core::ptr::addr_of_mut!(STAGE) };
    let len = if data.len() <= HEAD + TAIL {
        stage[..data.len()].copy_from_slice(data);
        data.len()
    } else {
        stage[..HEAD].copy_from_slice(&data[..HEAD]);
        stage[HEAD..HEAD + MARK.len()].copy_from_slice(MARK);
        let at = HEAD + MARK.len();
        let tail = &data[data.len() - TAIL..];
        stage[at..at + TAIL].copy_from_slice(tail);
        at + TAIL
    };
    let name = cstr16!("AnathemaBringup");
    if runtime::set_variable(name, &vendor(), attributes(), &stage[..len]).is_ok() {
        return true;
    }
    let short = len.min(4096);
    let from = len - short;
    let mut copy = [0u8; 4096];
    copy[..short].copy_from_slice(&stage[from..from + short]);
    runtime::set_variable(name, &vendor(), attributes(), &copy[..short]).is_ok()
}

pub fn clear() {
    let _ = runtime::delete_variable(cstr16!("AnathemaBringup"), &vendor());
}

pub fn take() -> Option<Vec<u8>> {
    let name = cstr16!("AnathemaBringup");
    let (data, _) = runtime::get_variable_boxed(name, &vendor()).ok()?;
    let out = data.to_vec();
    let _ = runtime::delete_variable(name, &vendor());
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
