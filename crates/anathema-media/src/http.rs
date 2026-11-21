use crate::readat::ReadAt;
use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use reqwest::{header, StatusCode};
use std::io;
use std::sync::Mutex;
use std::time::Duration;

const ATTEMPTS: u32 = 6;

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .user_agent("anathema-media/0.1")
        .build()?)
}

pub fn get_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs((1u64 << attempt).min(20)));
        }
        match client.get(url).send().and_then(|r| r.error_for_status()).and_then(|r| r.bytes()) {
            Ok(b) => return Ok(b.to_vec()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap()).with_context(|| format!("fetching {url}"))
}

#[derive(Debug, Clone)]
pub struct Head {
    pub len: u64,
    pub etag: Option<String>,
}

fn content_range_total(r: &Response) -> Option<u64> {
    let v = r.headers().get(header::CONTENT_RANGE)?.to_str().ok()?;
    v.rsplit('/').next()?.parse().ok()
}

pub fn probe(client: &Client, url: &str) -> Result<Head> {
    let r = client
        .get(url)
        .header(header::RANGE, "bytes=0-0")
        .header(header::ACCEPT_ENCODING, "identity")
        .send()?
        .error_for_status()?;
    if r.status() != StatusCode::PARTIAL_CONTENT {
        bail!("{url} does not serve byte ranges ({})", r.status());
    }
    let len = content_range_total(&r).context("no content range total")?;
    let etag = r.headers().get(header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned);
    Ok(Head { len, etag })
}

pub fn fetch_range(client: &Client, url: &str, start: u64, end: u64, etag: Option<&str>) -> Result<Vec<u8>> {
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs((1u64 << attempt).min(20)));
        }
        let mut req = client
            .get(url)
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={start}-{end}"));
        if let Some(e) = etag {
            req = req.header(header::IF_RANGE, e);
        }
        let res = req.send().map_err(anyhow::Error::from).and_then(|r| {
            if r.status() != StatusCode::PARTIAL_CONTENT {
                bail!("range answered {}", r.status());
            }
            Ok(r.bytes()?)
        });
        match res {
            Ok(b) if b.len() as u64 == end - start + 1 => return Ok(b.to_vec()),
            Ok(b) => last = Some(anyhow::anyhow!("short range {} of {}", b.len(), end - start + 1)),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap()).with_context(|| format!("range {start}-{end} of {url}"))
}

struct Chunk {
    start: u64,
    data: Vec<u8>,
}

pub struct Remote {
    client: Client,
    url: String,
    head: Head,
    chunk: Mutex<Chunk>,
}

impl Remote {
    pub fn open(client: Client, url: &str) -> Result<Self> {
        let head = probe(&client, url)?;
        Ok(Self {
            client,
            url: url.to_string(),
            head,
            chunk: Mutex::new(Chunk { start: 0, data: Vec::new() }),
        })
    }
    pub fn etag(&self) -> Option<&str> {
        self.head.etag.as_deref()
    }
}

impl ReadAt for Remote {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.head.len || buf.is_empty() {
            return Ok(0);
        }
        let mut c = self.chunk.lock().unwrap();
        let in_chunk = offset >= c.start && offset < c.start + c.data.len() as u64;
        if !in_chunk {
            let want = (buf.len() as u64).max(1 << 20).min(32 << 20);
            let end = (offset + want).min(self.head.len) - 1;
            let data = fetch_range(&self.client, &self.url, offset, end, self.head.etag.as_deref())
                .map_err(|e| io::Error::other(format!("{e:#}")))?;
            c.start = offset;
            c.data = data;
        }
        let o = (offset - c.start) as usize;
        let n = buf.len().min(c.data.len() - o);
        buf[..n].copy_from_slice(&c.data[o..o + n]);
        Ok(n)
    }
    fn len(&self) -> u64 {
        self.head.len
    }
}
