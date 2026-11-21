use anyhow::{bail, ensure, Context, Result};
use hmac::Mac;
use sha2::Digest;
use std::io::{Read, Write};

type HmacSha256 = hmac::Hmac<sha2::Sha256>;
type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

pub struct Header {
    pub raw: Vec<u8>,
    pub profile: u32,
    pub fields: Vec<(String, Vec<u8>)>,
}

impl Header {
    pub fn field(&self, name: &str) -> Option<&[u8]> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_slice())
    }
    pub fn key_url(&self) -> Option<String> {
        self.field("com.apple.wkms.fcs-key-url").map(|v| String::from_utf8_lossy(v).into_owned())
    }
}

pub fn read_header(r: &mut impl Read) -> Result<Header> {
    let mut fixed = [0u8; 12];
    r.read_exact(&mut fixed).context("truncated AEA header")?;
    ensure!(&fixed[..4] == b"AEA1", "not an AEA1 archive");
    let profile = u32::from_le_bytes(fixed[4..8].try_into()?);
    let size = u32::from_le_bytes(fixed[8..12].try_into()?) as usize;
    ensure!(size <= 1 << 20, "AEA metadata too large");
    let mut raw = fixed.to_vec();
    raw.resize(12 + size, 0);
    r.read_exact(&mut raw[12..]).context("truncated AEA metadata")?;
    let mut fields = Vec::new();
    let mut p = 12;
    while p + 4 <= raw.len() {
        let len = u32::from_le_bytes(raw[p..p + 4].try_into()?) as usize;
        ensure!(len >= 5 && p + len <= raw.len(), "bad AEA field");
        let body = &raw[p + 4..p + len];
        let z = body.iter().position(|&b| b == 0).context("AEA field without name end")?;
        fields.push((String::from_utf8_lossy(&body[..z]).into_owned(), body[z + 1..].to_vec()));
        p += len;
    }
    Ok(Header { raw, profile, fields })
}

pub fn check_key_url(url: &str) -> Result<()> {
    let rest = url.strip_prefix("https://").context("key url is not https")?;
    let (host, path) = rest.split_once('/').context("key url has no path")?;
    ensure!(
        host == "wkms-public.apple.com" || host == "fcs-keys-pub-prod.cdn-apple.com",
        "refusing key server {host}"
    );
    ensure!(
        path.starts_with("fcs-keys/") && !path.contains(['?', '#', '@']),
        "unexpected key path"
    );
    Ok(())
}

pub fn fetch_key_pem(h: &Header) -> Result<String> {
    let url = h.key_url().context("archive names no fcs key url")?;
    check_key_url(&url)?;
    let client = crate::http::client()?;
    let body = crate::http::get_bytes(&client, &url)?;
    let pem = String::from_utf8(body).context("key is not text")?;
    ensure!(pem.contains("BEGIN PRIVATE KEY"), "unexpected key response");
    Ok(pem)
}

pub fn archive_key(h: &Header, pem: &str) -> Result<[u8; 32]> {
    use base64::Engine;
    use hpke::{aead::AesGcm256, kdf::HkdfSha256, kem::DhP256HkdfSha256, Deserializable, OpModeR};
    use p256::pkcs8::DecodePrivateKey;
    let resp = h.field("com.apple.wkms.fcs-response").context("archive has no fcs response")?;
    let json: serde_json::Value = serde_json::from_slice(resp)?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let enc = b64.decode(json["enc-request"].as_str().context("no enc-request")?)?;
    let wrapped = b64.decode(json["wrapped-key"].as_str().context("no wrapped-key")?)?;
    let secret = p256::SecretKey::from_pkcs8_pem(pem).context("bad p256 key")?;
    let sk = <DhP256HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(&secret.to_bytes())
        .map_err(|e| anyhow::anyhow!("hpke private key: {e:?}"))?;
    let encapped = <DhP256HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&enc)
        .map_err(|e| anyhow::anyhow!("hpke encapped key: {e:?}"))?;
    let key = hpke::single_shot_open::<AesGcm256, HkdfSha256, DhP256HkdfSha256>(
        &OpModeR::Base,
        &sk,
        &encapped,
        &[],
        &wrapped,
        &[],
    )
    .map_err(|e| anyhow::anyhow!("cannot unwrap archive key: {e:?}"))?;
    key.try_into().map_err(|_| anyhow::anyhow!("archive key is not 32 bytes"))
}

fn hkdf32(ikm: &[u8], salt: Option<&[u8]>, info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(salt, ikm).expand(info, &mut out).unwrap();
    out
}

struct Keys([u8; 80]);

impl Keys {
    fn derive(ikm: &[u8], info: &[u8]) -> Self {
        let mut out = [0u8; 80];
        hkdf::Hkdf::<sha2::Sha256>::new(None, ikm).expand(info, &mut out).unwrap();
        Self(out)
    }
    fn mac(&self, parts: &[&[u8]], salt_len: usize) -> [u8; 32] {
        let mut m = HmacSha256::new_from_slice(&self.0[..32]).unwrap();
        for p in parts {
            m.update(p);
        }
        m.update(&(salt_len as u64).to_le_bytes());
        m.finalize().into_bytes().into()
    }
    fn apply(&self, data: &mut [u8]) {
        use ctr::cipher::{KeyIvInit, StreamCipher};
        let key: &[u8; 32] = self.0[32..64].try_into().unwrap();
        let iv: &[u8; 16] = self.0[64..].try_into().unwrap();
        Aes256Ctr::new(key.into(), iv.into()).apply_keystream(data);
    }
}

fn label(name: &str, index: u32) -> Vec<u8> {
    [name.as_bytes(), &index.to_le_bytes()].concat()
}

fn take<const N: usize>(r: &mut impl Read, what: &str) -> Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b).with_context(|| format!("truncated AEA {what}"))?;
    Ok(b)
}

fn take_vec(r: &mut impl Read, n: usize, what: &str) -> Result<Vec<u8>> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).with_context(|| format!("truncated AEA {what}"))?;
    Ok(b)
}

fn inflate(alg: u8, data: Vec<u8>, raw: usize) -> Result<Vec<u8>> {
    if data.len() == raw {
        return Ok(data);
    }
    match alg {
        b'e' | b'f' => crate::codec::lzvn_or_lzfse(&data, raw),
        _ => bail!("stored segment has wrong size"),
    }
}

pub fn decrypt(
    key: &[u8; 32],
    input: &mut impl Read,
    out: &mut impl Write,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<u64> {
    let header = read_header(input)?;
    let salt: [u8; 32] = take(input, "salt")?;
    let main = hkdf32(key, Some(&salt), &label("AEA_AMK", header.profile));
    let root_mac: [u8; 32] = take(input, "root mac")?;
    let mut root: [u8; 48] = take(input, "root header")?;
    let mut expected: [u8; 32] = take(input, "cluster mac")?;
    let rk = Keys::derive(&main, b"AEA_RHEK");
    let auth = &header.raw[12..];
    let want = rk.mac(&[&expected, auth, &root], expected.len() + auth.len());
    ensure!(want == root_mac, "root header fails its mac: wrong key for this archive");
    rk.apply(&mut root);
    let file_bytes = u64::from_le_bytes(root[..8].try_into()?);
    let seg_bytes = u32::from_le_bytes(root[16..20].try_into()?) as u64;
    let per_cluster = u32::from_le_bytes(root[20..24].try_into()?) as usize;
    let alg = root[24];
    let checksum = match root[25] {
        0 => 0usize,
        2 => 32,
        other => bail!("unsupported segment checksum kind {other}"),
    };
    ensure!(seg_bytes > 0 && seg_bytes <= 64 << 20 && per_cluster > 0 && per_cluster <= 1 << 16, "bad root geometry");
    let hb = 8 + checksum;
    let mut written = 0u64;
    let mut cluster = 0u32;
    loop {
        let ck = hkdf32(&main, None, &label("AEA_CK", cluster));
        let hk = Keys::derive(&ck, b"AEA_CHEK");
        let mut headers = take_vec(input, hb * per_cluster, "segment headers")?;
        let next: [u8; 32] = take(input, "next mac")?;
        let macs = take_vec(input, 32 * per_cluster, "segment macs")?;
        let want = hk.mac(&[&next, &macs, &headers], 32 + macs.len());
        ensure!(want == expected, "cluster {cluster} fails its mac");
        hk.apply(&mut headers);
        for (i, h) in headers.chunks_exact(hb).enumerate() {
            let raw = u32::from_le_bytes(h[..4].try_into()?) as usize;
            let stored = u32::from_le_bytes(h[4..8].try_into()?) as usize;
            if raw == 0 {
                continue;
            }
            ensure!(
                raw as u64 <= seg_bytes && stored <= 64 << 20 && written + raw as u64 <= file_bytes,
                "segment {i} of cluster {cluster} is out of range"
            );
            let mut data = take_vec(input, stored, "segment")?;
            let sk = Keys::derive(&ck, &label("AEA_SK", i as u32));
            ensure!(sk.mac(&[&data], 0) == macs[i * 32..i * 32 + 32], "segment {i} of cluster {cluster} fails its mac");
            sk.apply(&mut data);
            let plain = inflate(alg, data, raw)?;
            if checksum == 32 {
                ensure!(sha2::Sha256::digest(&plain)[..] == h[8..40], "segment checksum mismatch");
            }
            out.write_all(&plain)?;
            written += raw as u64;
            progress(written, file_bytes);
        }
        expected = next;
        cluster += 1;
        if written >= file_bytes {
            break;
        }
    }
    ensure!(written == file_bytes, "archive holds {written} bytes, expected {file_bytes}");
    out.flush()?;
    Ok(written)
}
