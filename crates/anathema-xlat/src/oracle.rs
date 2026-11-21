use crate::decode::*;
use crate::run::*;
use crate::trans::*;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub const DATA: u64 = 0x1000_0000;
pub const CODE: u64 = 0x1001_0000;
pub const BASEREG: u64 = DATA + 0x8000;
pub const WIN_ADDR: u64 = BASEREG - 512;
pub const WIN_LEN: usize = 1024;
pub const MAXW: usize = 24;
pub const QOFF: usize = 272;
pub const WOFF: usize = QOFF + 512;
pub const COFF: usize = WOFF + WIN_LEN;
pub const INSZ: usize = COFF + 4 * (MAXW + 1);
pub const EPI: u64 = 0x1010_1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub x: [u64; 31],
    pub sp: u64,
    pub nzcv: u64,
    pub q: [[u8; 16]; 32],
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn val(&mut self) -> u64 {
        match self.below(14) {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => (1 << 63) - 1,
            5 => 0x8000_0000,
            6 => 0x7fff_ffff,
            7 => 0xffff_ffff,
            8 => 1u64 << self.below(64),
            9 => (1u64 << self.below(64)).wrapping_sub(1),
            10 => (1u64 << self.below(64)).wrapping_add(1),
            11 => self.below(256),
            12 => (self.below(256)).wrapping_neg(),
            _ => self.next(),
        }
    }
}

const ASM: &str = r#"
.equ CODE, 0x10010000
.equ WIN, 0x10007e00
.equ INBUF, 0x10200000
.equ SAVE, 0x10201000
.section .text,"ax"
.globl _start
_start:
 ldr x19, =INBUF
 mov x20, #INSZ
 mov x21, #0
1: mov x0, #0
 add x1, x19, x21
 sub x2, x20, x21
 mov x8, #63
 svc #0
 cmp x0, #0
 b.le 9f
 add x21, x21, x0
 cmp x21, x20
 b.lt 1b
 ldr x1, =WIN
 add x2, x19, #WOFFV
 mov x3, #128
2: ldr x4, [x2], #8
 str x4, [x1], #8
 subs x3, x3, #1
 b.ne 2b
 ldr x1, =CODE
 add x2, x19, #COFFV
 mov x3, #CODEW
3: ldr w4, [x2], #4
 str w4, [x1], #4
 subs x3, x3, #1
 b.ne 3b
 ldr x1, =CODE
 mov x3, #3
4: dc cvau, x1
 ic ivau, x1
 add x1, x1, #64
 subs x3, x3, #1
 b.ne 4b
 dsb ish
 isb
 mov x30, x19
 msr fpcr, xzr
 ldp q0, q1, [x30, #272]
 ldp q2, q3, [x30, #304]
 ldp q4, q5, [x30, #336]
 ldp q6, q7, [x30, #368]
 ldp q8, q9, [x30, #400]
 ldp q10, q11, [x30, #432]
 ldp q12, q13, [x30, #464]
 ldp q14, q15, [x30, #496]
 ldp q16, q17, [x30, #528]
 ldp q18, q19, [x30, #560]
 ldp q20, q21, [x30, #592]
 ldp q22, q23, [x30, #624]
 ldp q24, q25, [x30, #656]
 ldp q26, q27, [x30, #688]
 ldp q28, q29, [x30, #720]
 ldp q30, q31, [x30, #752]
 ldr x0, [x30, #256]
 msr nzcv, x0
 ldr x0, [x30, #248]
 mov sp, x0
 ldp x0, x1, [x30, #0]
 ldp x2, x3, [x30, #16]
 ldp x4, x5, [x30, #32]
 ldp x6, x7, [x30, #48]
 ldp x8, x9, [x30, #64]
 ldp x10, x11, [x30, #80]
 ldp x12, x13, [x30, #96]
 ldp x14, x15, [x30, #112]
 ldp x16, x17, [x30, #128]
 ldp x18, x19, [x30, #144]
 ldp x20, x21, [x30, #160]
 ldp x22, x23, [x30, #176]
 ldp x24, x25, [x30, #192]
 ldp x26, x27, [x30, #208]
 ldp x28, x29, [x30, #224]
 ldr x30, [x30, #240]
 b codeaddr
9: mov x0, #0
 mov x8, #93
 svc #0
.ltorg
.section .epi,"ax"
.globl epi
epi:
 msr tpidr_el0, x30
 ldr x30, =SAVE
 stp x0, x1, [x30, #0]
 stp x2, x3, [x30, #16]
 stp x4, x5, [x30, #32]
 stp x6, x7, [x30, #48]
 stp x8, x9, [x30, #64]
 stp x10, x11, [x30, #80]
 stp x12, x13, [x30, #96]
 stp x14, x15, [x30, #112]
 stp x16, x17, [x30, #128]
 stp x18, x19, [x30, #144]
 stp x20, x21, [x30, #160]
 stp x22, x23, [x30, #176]
 stp x24, x25, [x30, #192]
 stp x26, x27, [x30, #208]
 stp x28, x29, [x30, #224]
 mrs x0, tpidr_el0
 str x0, [x30, #240]
 mov x0, sp
 str x0, [x30, #248]
 mrs x0, nzcv
 str x0, [x30, #256]
 stp q0, q1, [x30, #272]
 stp q2, q3, [x30, #304]
 stp q4, q5, [x30, #336]
 stp q6, q7, [x30, #368]
 stp q8, q9, [x30, #400]
 stp q10, q11, [x30, #432]
 stp q12, q13, [x30, #464]
 stp q14, q15, [x30, #496]
 stp q16, q17, [x30, #528]
 stp q18, q19, [x30, #560]
 stp q20, q21, [x30, #592]
 stp q22, q23, [x30, #624]
 stp q24, q25, [x30, #656]
 stp q26, q27, [x30, #688]
 stp q28, q29, [x30, #720]
 stp q30, q31, [x30, #752]
 mov x0, #1
 mov x1, x30
 mov x2, #784
 mov x8, #64
 svc #0
 ldr x1, =WIN
 mov x0, #1
 mov x2, #1024
 mov x8, #64
 svc #0
 b _start
.ltorg
.section .databuf,"awx",%progbits
.skip 0x10000
.section .codebuf,"awx",%progbits
.skip 0x1000
.section .work,"awx",%progbits
.skip 0x2000
"#;

const LDS: &str = r#"
SECTIONS {
 . = 0x10000000; .databuf : { *(.databuf) }
 . = 0x10010000; .codebuf : { *(.codebuf) }
 . = 0x10100000; .text : { *(.text) }
 . = 0x10101000; .epi : { *(.epi) }
 . = 0x10200000; .work : { *(.work) }
}
codeaddr = 0x10010000;
ENTRY(_start)
"#;

pub struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    elf: String,
}

fn build_elf(dir: &str) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let s = format!("{}/t.s", dir);
    let l = format!("{}/t.ld", dir);
    let o = format!("{}/t.o", dir);
    let e = format!("{}/t.elf", dir);
    let asm = ASM.replace("INSZ", &INSZ.to_string()).replace("WOFFV", &WOFF.to_string()).replace("COFFV", &COFF.to_string()).replace("CODEW", &(MAXW + 1).to_string());
    std::fs::write(&s, asm).map_err(|e| e.to_string())?;
    std::fs::write(&l, LDS).map_err(|e| e.to_string())?;
    let st = Command::new("aarch64-linux-gnu-as").args([&s, "-o", &o]).status().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("as failed".into());
    }
    let st = Command::new("aarch64-linux-gnu-ld")
        .args(["-z", "max-page-size=0x1000", "-z", "common-page-size=0x1000", "-static", "-nostdlib", "-T", &l, &o, "-o", &e])
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("ld failed".into());
    }
    Ok(e)
}

fn spawn(elf: &str) -> Result<(Child, ChildStdin, ChildStdout), String> {
    let mut c = Command::new("qemu-aarch64")
        .arg(elf)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let i = c.stdin.take().unwrap();
    let o = c.stdout.take().unwrap();
    Ok((c, i, o))
}

impl Oracle {
    pub fn new(dir: &str) -> Result<Oracle, String> {
        let elf = build_elf(dir)?;
        let (child, stdin, stdout) = spawn(&elf)?;
        Ok(Oracle { child, stdin, stdout, elf })
    }

    pub fn run(&mut self, st: &State, win: &[u8], words: &[u32]) -> Result<(State, Vec<u8>), String> {
        let mut inb = vec![0u8; INSZ];
        for i in 0..31 {
            inb[i * 8..i * 8 + 8].copy_from_slice(&st.x[i].to_le_bytes());
        }
        inb[248..256].copy_from_slice(&st.sp.to_le_bytes());
        inb[256..264].copy_from_slice(&st.nzcv.to_le_bytes());
        for i in 0..32 {
            inb[QOFF + i * 16..QOFF + i * 16 + 16].copy_from_slice(&st.q[i]);
        }
        inb[WOFF..WOFF + WIN_LEN].copy_from_slice(win);
        let n = words.len();
        let off = COFF;
        for (i, w) in words.iter().enumerate() {
            inb[off + i * 4..off + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let brw = encode_b(CODE + 4 * n as u64, EPI);
        inb[off + n * 4..off + n * 4 + 4].copy_from_slice(&brw.to_le_bytes());
        let r = self.stdin.write_all(&inb).and_then(|_| self.stdin.flush());
        let mut out = vec![0u8; 784 + WIN_LEN];
        let r = r.and_then(|_| self.stdout.read_exact(&mut out));
        if let Err(e) = r {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let (c, i, o) = spawn(&self.elf)?;
            self.child = c;
            self.stdin = i;
            self.stdout = o;
            return Err(format!("oracle died: {}", e));
        }
        let mut s = State { x: [0; 31], sp: 0, nzcv: 0, q: [[0; 16]; 32] };
        for i in 0..31 {
            s.x[i] = u64::from_le_bytes(out[i * 8..i * 8 + 8].try_into().unwrap());
        }
        for i in 0..32 {
            s.q[i].copy_from_slice(&out[QOFF + i * 16..QOFF + i * 16 + 16]);
        }
        s.sp = u64::from_le_bytes(out[248..256].try_into().unwrap());
        s.nzcv = u64::from_le_bytes(out[256..264].try_into().unwrap()) & 0xf000_0000;
        Ok((s, out[784..].to_vec()))
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn encode_b(from: u64, to: u64) -> u32 {
    let d = (to as i64 - from as i64) / 4;
    0x1400_0000 | (d as u32 & 0x03ff_ffff)
}

pub struct Native {
    pub runner: Runner,
    _data: Region,
    _code: Region,
}

impl Native {
    pub fn new(mem: MemModel) -> Native {
        let mut cfg = Config::new(mem);
        cfg.pc_base = CODE;
        let data = Region::fixed(DATA as usize, 0x10000, false).expect("map data");
        let code = Region::fixed(CODE as usize, 0x1000, false).expect("map code");
        Native { runner: Runner::new(cfg), _data: data, _code: code }
    }

    pub fn run(&mut self, st: &State, win: &[u8], words: &[u32]) -> Result<(State, Vec<u8>), String> {
        unsafe {
            std::ptr::write_bytes(DATA as *mut u8, 0, 0x10000);
            std::ptr::copy_nonoverlapping(win.as_ptr(), WIN_ADDR as *mut u8, WIN_LEN);
            for (i, w) in words.iter().enumerate() {
                *((CODE as *mut u32).add(i)) = *w;
            }
        }
        self.runner.reset_cache();
        let mut ctx = Ctx::default();
        for i in 0..31 {
            ctx.x[i] = st.x[i];
        }
        ctx.q = st.q;
        ctx.x[31] = st.sp;
        ctx.pc = CODE;
        ctx.flags = nzcv_to_flags(st.nzcv, self.runner.cfg.eflags_base as u64);
        let w = Words { base: CODE, words };
        let end = CODE + 4 * words.len() as u64;
        self.runner.run(&mut ctx, &w, end, 100)?;
        let mut s = State { x: [0; 31], sp: ctx.x[31], nzcv: flags_to_nzcv(ctx.flags), q: ctx.q };
        s.x.copy_from_slice(&ctx.x[..31]);
        let mut win_out = vec![0u8; WIN_LEN];
        unsafe {
            std::ptr::copy_nonoverlapping(WIN_ADDR as *const u8, win_out.as_mut_ptr(), WIN_LEN);
        }
        Ok((s, win_out))
    }
}

const PATS: &[(u32, u32, u32)] = &[
    (0x1100_0000, 0x1f80_0000, 10),
    (0x1200_0000, 0x1f80_0000, 8),
    (0x1280_0000, 0x1f80_0000, 4),
    (0x1300_0000, 0x1f80_0000, 8),
    (0x1380_0000, 0x1f80_0000, 3),
    (0x1000_0000, 0x1f00_0000, 2),
    (0x0a00_0000, 0x1f00_0000, 8),
    (0x0b00_0000, 0x1f20_0000, 10),
    (0x0b20_0000, 0x1fe0_0000, 6),
    (0x1a00_0000, 0x1fe0_fc00, 4),
    (0x3a40_0000, 0x3fe0_0c10, 4),
    (0x3a40_0800, 0x3fe0_0c10, 4),
    (0x1a80_0000, 0x3fe0_0800, 6),
    (0x1b00_0000, 0x1f00_0000, 6),
    (0x1ac0_0000, 0x7fe0_0000, 6),
    (0x5ac0_0000, 0x7fe0_0000, 4),
    (0x3900_0000, 0x3f00_0000, 5),
    (0x3800_0000, 0x3f20_0c00, 3),
    (0x3800_0c00, 0x3f20_0c00, 3),
    (0x3800_0400, 0x3f20_0c00, 3),
    (0x3820_0800, 0x3f20_0c00, 4),
    (0x2800_0000, 0x3e00_0000, 5),
    (0x1800_0000, 0x3f00_0000, 1),
    (0x0800_0000, 0x3f00_0000, 3),
    (0x3820_0000, 0x3f20_0c00, 4),
    (0xd503_3000, 0xffff_f000, 1),
    (0x5400_0000, 0xff00_0010, 3),
    (0x3400_0000, 0x7e00_0000, 2),
    (0x3600_0000, 0x7e00_0000, 2),
    (0x1400_0000, 0xfc00_0000, 1),
    (0x3d00_0000, 0x3f00_0000, 6),
    (0x3c00_0000, 0x3f20_0c00, 3),
    (0x3c00_0c00, 0x3f20_0c00, 2),
    (0x3c00_0400, 0x3f20_0c00, 2),
    (0x3c20_0800, 0x3f20_0c00, 3),
    (0x2c00_0000, 0x3e00_0000, 5),
    (0x1c00_0000, 0x3f00_0000, 1),
    (0x1e20_0800, 0xff20_0c00, 8),
    (0x1e20_4000, 0xff20_7c00, 4),
    (0x1e20_2000, 0xff20_fc07, 3),
    (0x1e20_0c00, 0xff20_0c00, 2),
    (0x1e20_1000, 0xff20_1fe0, 1),
    (0x1f00_0000, 0xff00_0000, 3),
    (0x1e20_0000, 0x7f20_fc00, 6),
    (0x0e20_1c00, 0x9f20_fc00, 5),
    (0x0e20_8400, 0x9f20_fc00, 3),
    (0x0f00_0400, 0x9ff8_0c00, 3),
    (0x0e00_0c00, 0xbfe0_fc00, 2),
    (0x0e00_3c00, 0xbfe0_fc00, 2),
    (0x4e00_1c00, 0xffe0_fc00, 2),
];

fn dests(i: &Insn) -> Vec<u8> {
    match *i {
        Insn::Adr { rd, .. } | Insn::Adrp { rd, .. } => vec![rd],
        Insn::AddSubImm { rd, .. }
        | Insn::AddSubSh { rd, .. }
        | Insn::AddSubExt { rd, .. }
        | Insn::AddSubC { rd, .. }
        | Insn::LogImm { rd, .. }
        | Insn::LogSh { rd, .. }
        | Insn::MovW { rd, .. }
        | Insn::Bfm { rd, .. }
        | Insn::Extr { rd, .. }
        | Insn::ShV { rd, .. }
        | Insn::Madd { rd, .. }
        | Insn::Mull { rd, .. }
        | Insn::Mulh { rd, .. }
        | Insn::Div { rd, .. }
        | Insn::Csel { rd, .. }
        | Insn::Clz { rd, .. }
        | Insn::Cls { rd, .. }
        | Insn::Rbit { rd, .. }
        | Insn::Rev { rd, .. }
        | Insn::Crc { rd, .. } => vec![rd],
        Insn::Ld { rt, .. } | Insn::Ldex { rt, .. } | Insn::Ldar { rt, .. } => vec![rt],
        Insn::Ldp { rt, rt2, .. } => vec![rt, rt2],
        Insn::Stex { rs, .. } => vec![rs],
        Insn::Atomic { rt, rs, .. } => vec![rt, rs],
        Insn::Cas { rs, .. } => vec![rs],
        Insn::Pac { zero: Some(r) } => vec![r],
        _ => vec![],
    }
}

fn mem_fix(w: u32, i: &Insn) -> Option<u32> {
    let mut w = w;
    match *i {
        Insn::Ld { addr: Addr::Lit { .. }, .. } => {
            return Some(w);
        }
        Insn::Ld { addr, .. } | Insn::St { addr, .. } => {
            w = (w & !(0x1f << 5)) | (28 << 5);
            if let Addr::Reg { .. } = addr {
                w = (w & !(0x1f << 16)) | (27 << 16);
            }
        }
        Insn::VLd { addr: Addr::Lit { .. }, .. } => {
            return Some(w);
        }
        Insn::VLd { addr, .. } | Insn::VSt { addr, .. } => {
            w = (w & !(0x1f << 5)) | (28 << 5);
            if let Addr::Reg { .. } = addr {
                w = (w & !(0x1f << 16)) | (27 << 16);
            }
        }
        Insn::VLdp { .. } | Insn::VStp { .. } => {
            w = (w & !(0x1f << 5)) | (28 << 5);
        }
        Insn::Ldp { .. } | Insn::Stp { .. } | Insn::Ldex { .. } | Insn::Stex { .. } | Insn::Ldar { .. } | Insn::Stlr { .. } | Insn::Atomic { .. } | Insn::Cas { .. } => {
            w = (w & !(0x1f << 5)) | (28 << 5);
        }
        _ => {}
    }
    Some(w)
}

fn off_ok(i: &Insn) -> bool {
    match *i {
        Insn::Ld { addr: Addr::Imm { off, .. }, .. } | Insn::St { addr: Addr::Imm { off, .. }, .. } => off.abs() <= 400,
        Insn::Ldp { off, .. } | Insn::Stp { off, .. } | Insn::VLdp { off, .. } | Insn::VStp { off, .. } => off.abs() <= 400,
        Insn::VLd { addr: Addr::Imm { off, .. }, .. } | Insn::VSt { addr: Addr::Imm { off, .. }, .. } => off.abs() <= 400,
        _ => true,
    }
}

pub fn gen_program(rng: &mut Rng, n: usize) -> Vec<u32> {
    let total: u32 = PATS.iter().map(|p| p.2).sum();
    let mut out: Vec<u32> = Vec::new();
    while out.len() < n {
        let mut pick = rng.below(total as u64) as u32;
        let mut pat = PATS[0];
        for p in PATS {
            if pick < p.2 {
                pat = *p;
                break;
            }
            pick -= p.2;
        }
        let r = rng.next() as u32;
        let mut w = (pat.0 & pat.1) | (r & !pat.1);
        let idx = out.len();
        let remain = (n - idx) as u32;
        let ahead = 1 + rng.below(remain.min(5) as u64) as u32;
        if pat.0 == 0x1c00_0000 {
            if n < 6 {
                continue;
            }
            let tgt = rng.below(n as u64 - 5) as i32;
            let o = (tgt - idx as i32) as u32 & 0x7ffff;
            w = (w & !(0x7ffff << 5)) | (o << 5);
        } else if pat.0 == 0x5400_0000 || pat.0 == 0x3400_0000 {
            w = (w & !(0x7ffff << 5)) | (ahead << 5);
        } else if pat.0 == 0x3600_0000 {
            w = (w & !(0x3fff << 5)) | (ahead << 5);
        } else if pat.0 == 0x1400_0000 {
            w = (w & !0x03ff_ffff) | ahead;
        } else if pat.0 == 0x1800_0000 {
            if n < 2 {
                continue;
            }
            let tgt = rng.below(n as u64 - 1) as i32;
            let o = (tgt - idx as i32) as u32 & 0x7ffff;
            w = (w & !(0x7ffff << 5)) | (o << 5);
        }
        let i = decode(w);
        if matches!(i, Insn::Unsup(_)) || needs_interp(&i) {
            continue;
        }
        let w = match mem_fix(w, &i) {
            Some(x) => x,
            None => continue,
        };
        let i = decode(w);
        if matches!(i, Insn::Unsup(_)) || needs_interp(&i) || !off_ok(&i) {
            continue;
        }
        if dests(&i).iter().any(|&d| d == 27 || d == 28) {
            continue;
        }
        match i {
            Insn::Ldp { rt, rt2, rn, mode, .. } => {
                if rt == rt2 && rt != 31 || mode != Mode::Offset && (rt == rn || rt2 == rn) {
                    continue;
                }
            }
            Insn::Stp { rt, rt2, mode, rn, .. } => {
                if mode != Mode::Offset && (rt == rn || rt2 == rn) {
                    continue;
                }
            }
            Insn::Ld { rt, rn, addr: Addr::Imm { mode, .. }, .. } => {
                if mode != Mode::Offset && rt == rn {
                    continue;
                }
            }
            Insn::St { rt, rn, addr: Addr::Imm { mode, .. }, .. } => {
                if mode != Mode::Offset && rt == rn {
                    continue;
                }
            }
            Insn::Stex { rs, rt, rn, .. } => {
                if rs == rt || rs == rn {
                    continue;
                }
            }
            Insn::Ldar { .. } | Insn::Stlr { .. } => {}
            Insn::Ldex { size, .. } | Insn::Stex { size, .. } => {
                let _ = size;
            }
            _ => {}
        }
        out.push(w);
    }
    out
}

pub fn gen_state(rng: &mut Rng) -> (State, Vec<u8>) {
    let mut st = State { x: [0; 31], sp: rng.val(), nzcv: (rng.below(16)) << 28, q: [[0; 16]; 32] };
    for i in 0..32 {
        for h in 0..2 {
            let mut v: u64 = match rng.below(6) {
                0 => 0,
                1 => (1.0f64).to_bits(),
                2 => ((rng.below(2000) as f64 - 1000.0) / 7.0).to_bits(),
                3 => (rng.below(1 << 20) as f64 * 1.5e-3).to_bits() ^ (rng.below(2) << 63),
                4 => (f64::from_bits(rng.next()).clamp(-1e30, 1e30)).to_bits(),
                _ => rng.next(),
            };
            if (v >> 52) & 0x7ff == 0x7ff {
                v &= !(1 << 62);
            }
            let mut words = [v as u32, (v >> 32) as u32];
            for w in words.iter_mut() {
                if (*w >> 23) & 0xff == 0xff {
                    *w &= !(1 << 30);
                }
            }
            let v = words[0] as u64 | (words[1] as u64) << 32;
            st.q[i][h * 8..h * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
    }
    for i in 0..31 {
        st.x[i] = rng.val();
    }
    st.x[27] = rng.below(128);
    st.x[28] = BASEREG;
    let mut win = vec![0u8; WIN_LEN];
    for c in win.chunks_mut(8) {
        let v = rng.val().to_le_bytes();
        c.copy_from_slice(&v);
    }
    (st, win)
}

pub fn diff(a: &State, wa: &[u8], b: &State, wb: &[u8]) -> Option<String> {
    let mut s = String::new();
    for i in 0..31 {
        if a.x[i] != b.x[i] {
            s += &format!("x{}: oracle {:#x} native {:#x}\n", i, a.x[i], b.x[i]);
        }
    }
    if a.sp != b.sp {
        s += &format!("sp: oracle {:#x} native {:#x}\n", a.sp, b.sp);
    }
    if a.nzcv != b.nzcv {
        s += &format!("nzcv: oracle {:#x} native {:#x}\n", a.nzcv >> 28, b.nzcv >> 28);
    }
    for i in 0..32 {
        if a.q[i] != b.q[i] {
            let mut bad = false;
            for h in 0..2 {
                let la = u64::from_le_bytes(a.q[i][h * 8..h * 8 + 8].try_into().unwrap());
                let lb = u64::from_le_bytes(b.q[i][h * 8..h * 8 + 8].try_into().unwrap());
                if la != lb {
                    let nan64 = (la >> 52) & 0x7ff == 0x7ff && la << 12 != 0;
                    let nan32 = |w: u32| (w >> 23) & 0xff == 0xff && w << 9 != 0;
                    if !(nan64 || nan32(la as u32) || nan32((la >> 32) as u32)) {
                        bad = true;
                    }
                }
            }
            if bad {
                s += &format!("v{}: oracle {:x?} native {:x?}\n", i, a.q[i], b.q[i]);
            }
        }
    }
    if wa != wb {
        let k = wa.iter().zip(wb.iter()).position(|(x, y)| x != y).unwrap();
        s += &format!("mem differs at window+{}\n", k);
    }
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
