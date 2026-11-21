use sha1::Digest;
use std::io::Read;

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn sha256(data: &[u8]) -> String {
    hex(&sha2::Sha256::digest(data))
}

pub fn sha1_reader(mut r: impl Read, progress: &mut dyn FnMut(u64)) -> std::io::Result<String> {
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        progress(total);
    }
    Ok(hex(&h.finalize()))
}
