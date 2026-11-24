//! What identifies the target stick across the minutes the kernel files take to prepare.

/// Model, size, bus and partition layout (offset, length, type GUID). Drive letters are left
/// out: Windows may hand out new ones after a replug, and setup reads them fresh anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickId {
    pub model: String,
    pub bytes: u64,
    pub bus: u32,
    pub parts: Vec<(u64, u64, [u8; 16])>,
}

pub fn same_stick(before: &StickId, after: &StickId) -> Result<(), String> {
    if before == after {
        return Ok(());
    }
    Err("The selected disk changed while the kernel files were being prepared (unplugged or swapped?). Nothing was written; select the stick again.".into())
}
