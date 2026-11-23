//! Corpus file: a header, then one record per test case.
//!
//! All integers are little-endian. A record holds the program, the full input state and
//! memory window, and (if the program ran cleanly on the hardware) the full output state and
//! window. Inputs are stored rather than regenerated from a seed, so a corpus stays valid when
//! the generator in `anathema_xlat::oracle` changes.
//!
//! ```text
//! header:  b"AXHW0001"  data:u64  code:u64  host_len:u32  host:[u8]
//! record:  n:u32  words:[u32; n]  input:State  input_win:[u8; WIN_LEN]
//!          outcome:u8  (0 = ran, 1 = faulted, 2 = not deterministic)
//!          if outcome == 0: output:State  output_win:[u8; WIN_LEN]
//!          if outcome == 1: signal:u32  pc:u64 (offset of the faulting instruction from `code`)
//! State:   x:[u64; 31]  sp:u64  nzcv:u64  q:[[u8; 16]; 32]
//! ```

use anathema_xlat::oracle::{State, MAXW, WIN_LEN};
use std::io::{self, Read, Write};

const MAGIC: &[u8; 8] = b"AXHW0001";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Guest data region base (64 KiB). The window is at `data + 0x7e00`, x28 points at `data + 0x8000`.
    pub data: u64,
    /// Address of the first program word. Page aligned.
    pub code: u64,
    /// Free text describing the machine that recorded the corpus.
    pub host: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ran { state: State, win: Vec<u8> },
    Faulted { signal: u32, pc: u64 },
    NotDeterministic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub words: Vec<u32>,
    pub state: State,
    pub win: Vec<u8>,
    pub outcome: Outcome,
}

fn put_state(w: &mut impl Write, s: &State) -> io::Result<()> {
    for x in s.x {
        w.write_all(&x.to_le_bytes())?;
    }
    w.write_all(&s.sp.to_le_bytes())?;
    w.write_all(&s.nzcv.to_le_bytes())?;
    for q in &s.q {
        w.write_all(q)?;
    }
    Ok(())
}

fn u32_(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn u64_(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn get_state(r: &mut impl Read) -> io::Result<State> {
    let mut s = State { x: [0; 31], sp: 0, nzcv: 0, q: [[0; 16]; 32] };
    for x in s.x.iter_mut() {
        *x = u64_(r)?;
    }
    s.sp = u64_(r)?;
    s.nzcv = u64_(r)?;
    for q in s.q.iter_mut() {
        r.read_exact(q)?;
    }
    Ok(s)
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

pub fn write_header(w: &mut impl Write, h: &Header) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&h.data.to_le_bytes())?;
    w.write_all(&h.code.to_le_bytes())?;
    w.write_all(&(h.host.len() as u32).to_le_bytes())?;
    w.write_all(h.host.as_bytes())
}

pub fn read_header(r: &mut impl Read) -> io::Result<Header> {
    let mut magic = [0; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("not an anathema-hwcheck corpus (bad magic)"));
    }
    let data = u64_(r)?;
    let code = u64_(r)?;
    let n = u32_(r)? as usize;
    if n > 4096 {
        return Err(bad("host string too long"));
    }
    let mut host = vec![0; n];
    r.read_exact(&mut host)?;
    Ok(Header { data, code, host: String::from_utf8_lossy(&host).into_owned() })
}

pub fn write_case(w: &mut impl Write, c: &Case) -> io::Result<()> {
    w.write_all(&(c.words.len() as u32).to_le_bytes())?;
    for word in &c.words {
        w.write_all(&word.to_le_bytes())?;
    }
    put_state(w, &c.state)?;
    w.write_all(&c.win)?;
    match &c.outcome {
        Outcome::Ran { state, win } => {
            w.write_all(&[0])?;
            put_state(w, state)?;
            w.write_all(win)
        }
        Outcome::Faulted { signal, pc } => {
            w.write_all(&[1])?;
            w.write_all(&signal.to_le_bytes())?;
            w.write_all(&pc.to_le_bytes())
        }
        Outcome::NotDeterministic => w.write_all(&[2]),
    }
}

/// Reads the next case, or `Ok(None)` at a clean end of file.
pub fn read_case(r: &mut impl Read) -> io::Result<Option<Case>> {
    let mut b = [0; 4];
    match r.read_exact(&mut b) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(b) as usize;
    if n == 0 || n > MAXW {
        return Err(bad("program length out of range"));
    }
    let mut words = Vec::with_capacity(n);
    for _ in 0..n {
        words.push(u32_(r)?);
    }
    let state = get_state(r)?;
    let mut win = vec![0; WIN_LEN];
    r.read_exact(&mut win)?;
    let mut tag = [0; 1];
    r.read_exact(&mut tag)?;
    let outcome = match tag[0] {
        0 => {
            let state = get_state(r)?;
            let mut win = vec![0; WIN_LEN];
            r.read_exact(&mut win)?;
            Outcome::Ran { state, win }
        }
        1 => Outcome::Faulted { signal: u32_(r)?, pc: u64_(r)? },
        2 => Outcome::NotDeterministic,
        _ => return Err(bad("unknown outcome tag")),
    };
    Ok(Some(Case { words, state, win, outcome }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(seed: u64) -> State {
        let mut s = State { x: [0; 31], sp: seed, nzcv: 0x6000_0000, q: [[seed as u8; 16]; 32] };
        for (i, x) in s.x.iter_mut().enumerate() {
            *x = seed.wrapping_mul(i as u64 + 1);
        }
        s
    }

    #[test]
    fn round_trip() {
        let h = Header { data: 0x2_0000_0000, code: 0x2_0001_4000, host: "Apple M3 Pro".into() };
        let cases = vec![
            Case {
                words: vec![0x9100_0420, 0xd65f_03c0],
                state: state(1),
                win: vec![7; WIN_LEN],
                outcome: Outcome::Ran { state: state(2), win: vec![9; WIN_LEN] },
            },
            Case { words: vec![0], state: state(3), win: vec![0; WIN_LEN], outcome: Outcome::Faulted { signal: 4, pc: 0x1234 } },
            Case { words: vec![1, 2, 3], state: state(4), win: vec![1; WIN_LEN], outcome: Outcome::NotDeterministic },
        ];
        let mut buf = Vec::new();
        write_header(&mut buf, &h).unwrap();
        for c in &cases {
            write_case(&mut buf, c).unwrap();
        }
        let mut r = &buf[..];
        assert_eq!(read_header(&mut r).unwrap(), h);
        for c in &cases {
            assert_eq!(read_case(&mut r).unwrap().as_ref(), Some(c));
        }
        assert_eq!(read_case(&mut r).unwrap(), None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(read_header(&mut &b"NOTMAGIC0000000000000000"[..]).is_err());
        let mut r = &[0u8, 0, 0, 0][..]; // program length 0
        assert!(read_case(&mut r).is_err());
        let mut truncated = Vec::new();
        write_case(
            &mut truncated,
            &Case { words: vec![1], state: state(1), win: vec![0; WIN_LEN], outcome: Outcome::NotDeterministic },
        )
        .unwrap();
        truncated.truncate(truncated.len() - 10);
        assert!(read_case(&mut &truncated[..]).is_err());
    }
}
