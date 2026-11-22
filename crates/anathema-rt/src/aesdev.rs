use crate::mem::Ram;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes192, Aes256};
use alloc::vec::Vec;

pub struct AesState {
    key: [[u8; 32]; 2],
    key_len: [usize; 2],
    iv: [[u8; 16]; 4],
    encrypt: bool,
    cbc: bool,
    pub log: usize,
}

fn word_bytes(words: &[u32]) -> Vec<u8> {
    let mut v = Vec::new();
    for w in words {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v
}

impl AesState {
    pub fn new() -> AesState {
        AesState { key: [[0; 32]; 2], key_len: [0; 2], iv: [[0; 16]; 4], encrypt: false, cbc: false, log: 60 }
    }

    fn cmd_key(&mut self, fifo: &[u32]) -> bool {
        let cmd = fifo[0];
        let select = (cmd >> 24) & 7;
        let ctx = ((cmd >> 27) & 1) as usize;
        match (cmd >> 16) & 3 {
            0 => self.cbc = false,
            1 => self.cbc = true,
            _ => return false,
        }
        self.encrypt = (cmd >> 20) & 1 == 1;
        let len = [16usize, 24, 32, 64][((cmd >> 22) & 3) as usize];
        if select != 0 {
            let mut k = [0u8; 32];
            if (1..=3).contains(&select) {
                k[0..4].copy_from_slice(&select.to_le_bytes());
            }
            self.key[ctx] = k;
            self.key_len[ctx] = 32;
            true
        } else {
            if len > 32 || fifo.len() < len / 4 + 1 {
                return false;
            }
            let b = word_bytes(&fifo[1..1 + len / 4]);
            let mut k = [0u8; 32];
            k[..len].copy_from_slice(&b);
            self.key[ctx] = k;
            self.key_len[ctx] = len;
            true
        }
    }

    fn crypt(&self, ctx: usize, data: &mut [u8]) -> bool {
        let kl = self.key_len[ctx];
        let key = &self.key[ctx][..kl];
        macro_rules! run {
            ($c:ty) => {{
                let c = <$c>::new(GenericArray::from_slice(key));
                let mut prev = self.iv[0];
                for chunk in data.chunks_mut(16) {
                    if chunk.len() != 16 {
                        return false;
                    }
                    if self.encrypt {
                        if self.cbc {
                            for i in 0..16 {
                                chunk[i] ^= prev[i];
                            }
                        }
                        c.encrypt_block(GenericArray::from_mut_slice(chunk));
                        prev.copy_from_slice(chunk);
                    } else {
                        let mut cur = [0u8; 16];
                        cur.copy_from_slice(chunk);
                        c.decrypt_block(GenericArray::from_mut_slice(chunk));
                        if self.cbc {
                            for i in 0..16 {
                                chunk[i] ^= prev[i];
                            }
                        }
                        prev = cur;
                    }
                }
                true
            }};
        }
        match kl {
            16 => run!(Aes128),
            24 => run!(Aes192),
            32 => run!(Aes256),
            _ => false,
        }
    }

    pub fn process(&mut self, fifo: &mut Vec<u32>, ram: &Ram, irq_status: &mut u32, flag_info: &mut u32) {
        if fifo.is_empty() {
            return;
        }
        let cmd = fifo[0] >> 28;
        let ok = match cmd {
            1 => self.cmd_key(fifo),
            2 => {
                let ctx = ((fifo[0] & (3 << 27)) >> 26) as usize;
                if fifo.len() < 5 {
                    false
                } else if ctx < 4 {
                    let b = word_bytes(&fifo[1..5]);
                    self.iv[ctx].copy_from_slice(&b);
                    true
                } else {
                    true
                }
            }
            5 => {
                if fifo.len() < 4 {
                    false
                } else {
                    let ctx = ((fifo[0] >> 27) & 1) as usize;
                    let len = (fifo[0] & 0xff_ffff) as usize;
                    let src = fifo[2] as u64 | (((fifo[1] as u64) << 16) & 0xffff_0000_0000);
                    let dst = fifo[3] as u64 | (((fifo[1] as u64) << 32) & 0xffff_0000_0000);
                    if self.log > 0 {
                        self.log -= 1;
                        println!("  aes data len {} src {:#x} dst {:#x} key {} enc {} cbc {}", len, src, dst, self.key_len[ctx], self.encrypt, self.cbc);
                    }
                    if ram.contains(src, len as u64) && ram.contains(dst, len as u64) {
                        let mut buf = ram.bytes(src, len).to_vec();
                        if self.crypt(ctx, &mut buf) {
                            ram.bytes(dst, len).copy_from_slice(&buf);
                        }
                    }
                    true
                }
            }
            6 => {
                if fifo.len() < 2 {
                    false
                } else {
                    let ctx = ((fifo[0] & (3 << 27)) >> 26) as usize;
                    let addr = fifo[1] as u64 | (((fifo[0] as u64) << 32) & 0xff_0000_0000);
                    if ctx < 4 && ram.contains(addr, 16) {
                        ram.bytes(addr, 16).copy_from_slice(&self.iv[ctx]);
                    }
                    true
                }
            }
            8 => {
                if fifo[0] & (1 << 27) != 0 {
                    *irq_status |= 1 << 5;
                }
                *flag_info = fifo[0] & 0xff;
                true
            }
            _ => {
                *irq_status |= 1 << 2;
                false
            }
        };
        if ok || fifo.len() >= 9 || !(1..=8).contains(&cmd) {
            fifo.clear();
        }
    }
}
