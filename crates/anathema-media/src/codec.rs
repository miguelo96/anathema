use anyhow::{anyhow, bail, Result};

pub fn lzfse_stream(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    lzfse_rust::decode_bytes(data, &mut out).map_err(|e| anyhow!("lzfse: {e:?}"))?;
    Ok(out)
}

pub fn lzvn_raw(data: &[u8], raw: usize) -> Result<Vec<u8>> {
    let mut wrapped = Vec::with_capacity(data.len() + 16);
    wrapped.extend_from_slice(b"bvxn");
    wrapped.extend_from_slice(&(raw as u32).to_le_bytes());
    wrapped.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wrapped.extend_from_slice(data);
    wrapped.extend_from_slice(b"bvx$");
    lzfse_stream(&wrapped, raw)
}

pub fn has_bvx_magic(data: &[u8]) -> bool {
    data.len() >= 4 && matches!(&data[..4], b"bvx2" | b"bvx1" | b"bvxn" | b"bvx-")
}

pub fn lzvn_or_lzfse(data: &[u8], raw: usize) -> Result<Vec<u8>> {
    let out = if has_bvx_magic(data) { lzfse_stream(data, raw)? } else { lzvn_raw(data, raw)? };
    if out.len() != raw {
        bail!("decompressed to {} bytes, expected {raw}", out.len());
    }
    Ok(out)
}
