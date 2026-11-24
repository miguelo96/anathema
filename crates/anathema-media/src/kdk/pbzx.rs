//! pbzx: Apple's chunked xz container. After the magic and a u64 chunk size, each chunk is
//! `[u64 BE uncompressed size][u64 BE stored size][data]`; data is stored as is when the two
//! sizes match, otherwise it is a complete xz stream. Chunks are independent, so they are
//! decoded on worker threads and handed back in order.

use anyhow::{anyhow, bail, Result};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

const XZ_MAGIC: &[u8] = &[0xfd, b'7', b'z', b'X', b'Z', 0];

type Decoded = (u64, std::result::Result<Vec<u8>, String>);

pub struct Pbzx {
    results: Receiver<Decoded>,
    pending: BTreeMap<u64, std::result::Result<Vec<u8>, String>>,
    next: u64,
    buf: Vec<u8>,
    pos: usize,
    consumed: Arc<AtomicU64>,
    done: bool,
}

fn decode(usize_: u64, data: Vec<u8>) -> std::result::Result<Vec<u8>, String> {
    if data.len() as u64 == usize_ {
        return Ok(data);
    }
    if !data.starts_with(XZ_MAGIC) {
        return Err("pbzx chunk is neither stored nor xz".into());
    }
    let mut out = Vec::with_capacity(usize_ as usize);
    lzma_rs::xz_decompress(&mut &data[..], &mut out).map_err(|e| format!("pbzx xz chunk: {e}"))?;
    if out.len() as u64 != usize_ {
        return Err(format!("pbzx chunk decoded to {} bytes, header says {usize_}", out.len()));
    }
    Ok(out)
}

/// Reads chunks in order and queues them for the workers. Errors are reported in sequence.
fn read_chunks<R: Read>(mut inner: R, jobs: std::sync::mpsc::SyncSender<(u64, u64, Vec<u8>)>, results: Sender<Decoded>, consumed: Arc<AtomicU64>) {
    let mut seq = 0u64;
    loop {
        let mut hdr = [0u8; 16];
        let mut got = 0;
        while got < 16 {
            match inner.read(&mut hdr[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    let _ = results.send((seq, Err(format!("pbzx read: {e}"))));
                    return;
                }
            }
        }
        if got == 0 {
            return;
        }
        if got < 16 {
            let _ = results.send((seq, Err("pbzx chunk header truncated".into())));
            return;
        }
        let usize_ = u64::from_be_bytes(hdr[..8].try_into().unwrap());
        let csize = u64::from_be_bytes(hdr[8..].try_into().unwrap());
        if csize > 1 << 30 || usize_ > 1 << 30 {
            let _ = results.send((seq, Err(format!("pbzx chunk sizes {usize_}/{csize} are implausible"))));
            return;
        }
        let mut data = vec![0u8; csize as usize];
        if let Err(e) = inner.read_exact(&mut data) {
            let _ = results.send((seq, Err(format!("pbzx chunk truncated: {e}"))));
            return;
        }
        consumed.fetch_add(16 + csize, Ordering::Relaxed);
        if jobs.send((seq, usize_, data)).is_err() {
            return;
        }
        seq += 1;
    }
}

impl Pbzx {
    pub fn new<R: Read + Send + 'static>(mut inner: R, threads: usize) -> Result<Pbzx> {
        let mut head = [0u8; 12];
        inner.read_exact(&mut head).map_err(|e| anyhow!("pbzx header: {e}"))?;
        if &head[..4] != b"pbzx" {
            bail!("not a pbzx stream");
        }
        let consumed = Arc::new(AtomicU64::new(12));
        let threads = threads.max(1);
        let (job_tx, job_rx) = sync_channel::<(u64, u64, Vec<u8>)>(threads * 2);
        let (res_tx, res_rx) = channel::<Decoded>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for _ in 0..threads {
            let job_rx = job_rx.clone();
            let res_tx = res_tx.clone();
            thread::spawn(move || loop {
                let job = job_rx.lock().map(|rx| rx.recv());
                let Ok(Ok((seq, usize_, data))) = job else { return };
                if res_tx.send((seq, decode(usize_, data))).is_err() {
                    return;
                }
            });
        }
        let c = consumed.clone();
        thread::spawn(move || read_chunks(inner, job_tx, res_tx, c));
        Ok(Pbzx { results: res_rx, pending: BTreeMap::new(), next: 0, buf: Vec::new(), pos: 0, consumed, done: false })
    }

    /// Compressed bytes read from the input so far (for progress).
    pub fn consumed(&self) -> Arc<AtomicU64> {
        self.consumed.clone()
    }

    fn refill(&mut self) -> io::Result<bool> {
        loop {
            if let Some(r) = self.pending.remove(&self.next) {
                self.next += 1;
                return match r {
                    Ok(b) => {
                        self.buf = b;
                        self.pos = 0;
                        Ok(true)
                    }
                    Err(e) => {
                        self.done = true;
                        Err(io::Error::new(io::ErrorKind::InvalidData, e))
                    }
                };
            }
            match self.results.recv() {
                Ok((seq, r)) => {
                    self.pending.insert(seq, r);
                }
                Err(_) => {
                    self.done = true;
                    return if self.pending.is_empty() {
                        Ok(false)
                    } else {
                        Err(io::Error::new(io::ErrorKind::InvalidData, "pbzx chunk missing"))
                    };
                }
            }
        }
    }
}

impl Read for Pbzx {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            if self.done || !self.refill()? {
                return Ok(0);
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}
