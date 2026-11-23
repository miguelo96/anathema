//! Runs generated arm64 programs directly on an Apple silicon Mac and captures the result.
//!
//! How one case runs:
//! 1. The program is copied into a MAP_JIT page at a 16K-aligned address, preceded by an
//!    entry stub and followed by `brk`. The stub is assembled into this binary
//!    (`global_asm!` below) and copied in front of the program, so it falls straight through
//!    into it.
//! 2. The stub saves the host's callee-saved state into a block, loads the complete guest
//!    state (x0-x30, sp, nzcv, q0-q31, FPCR = 0) and runs the program.
//! 3. The program ends at `brk`, or faults earlier. Either way the kernel delivers a signal.
//!    The handler (on its own stack, since the guest sp is random) copies the guest state out
//!    of the signal context, then rewrites that context so that sigreturn resumes the host
//!    right after the call into the stub, as if the stub had returned.
//!
//! Memory layout matches `anathema_xlat::oracle`: a 64 KiB data region with the 1 KiB window
//! at +0x7e00 and x28 pointing at +0x8000. Only the base addresses differ, and they are
//! recorded in the corpus header so the replay can use the same ones.

use crate::corpus::{Case, Header, Outcome};
use anathema_xlat::oracle::{State, WIN_ADDR, DATA, WIN_LEN};
use std::ptr;

// Block offsets shared by the stub and the signal handler.
const B_X: usize = 0; // guest x0..x30
const B_SP: usize = 248; // guest sp
const B_NZCV: usize = 256; // guest nzcv
const B_HX18: usize = 264; // host x18..x29
const B_HX30: usize = 360; // host x30 (return address into Rust)
const B_HSP: usize = 368; // host sp
const B_HFPCR: usize = 376; // host fpcr
const B_HD8: usize = 384; // host d8..d15
const B_Q: usize = 448; // guest q0..q31
const B_SIZE: usize = 1024;

std::arch::global_asm!(
    ".text",
    ".p2align 2",
    ".globl _axhw_entry_start",
    "_axhw_entry_start:",
    // save host state
    "stp x18, x19, [x0, #264]",
    "stp x20, x21, [x0, #280]",
    "stp x22, x23, [x0, #296]",
    "stp x24, x25, [x0, #312]",
    "stp x26, x27, [x0, #328]",
    "stp x28, x29, [x0, #344]",
    "str x30, [x0, #360]",
    "mov x1, sp",
    "str x1, [x0, #368]",
    "mrs x1, fpcr",
    "str x1, [x0, #376]",
    "stp d8, d9, [x0, #384]",
    "stp d10, d11, [x0, #400]",
    "stp d12, d13, [x0, #416]",
    "stp d14, d15, [x0, #432]",
    // load guest state
    "msr fpcr, xzr",
    "ldp q0, q1, [x0, #448]",
    "ldp q2, q3, [x0, #480]",
    "ldp q4, q5, [x0, #512]",
    "ldp q6, q7, [x0, #544]",
    "ldp q8, q9, [x0, #576]",
    "ldp q10, q11, [x0, #608]",
    "ldp q12, q13, [x0, #640]",
    "ldp q14, q15, [x0, #672]",
    "ldp q16, q17, [x0, #704]",
    "ldp q18, q19, [x0, #736]",
    "ldp q20, q21, [x0, #768]",
    "ldp q22, q23, [x0, #800]",
    "ldp q24, q25, [x0, #832]",
    "ldp q26, q27, [x0, #864]",
    "ldp q28, q29, [x0, #896]",
    "ldp q30, q31, [x0, #928]",
    "ldr x1, [x0, #256]",
    "msr nzcv, x1",
    "ldr x1, [x0, #248]",
    "mov sp, x1",
    "mov x30, x0",
    "ldp x0, x1, [x30, #0]",
    "ldp x2, x3, [x30, #16]",
    "ldp x4, x5, [x30, #32]",
    "ldp x6, x7, [x30, #48]",
    "ldp x8, x9, [x30, #64]",
    "ldp x10, x11, [x30, #80]",
    "ldp x12, x13, [x30, #96]",
    "ldp x14, x15, [x30, #112]",
    "ldp x16, x17, [x30, #128]",
    "ldp x18, x19, [x30, #144]",
    "ldp x20, x21, [x30, #160]",
    "ldp x22, x23, [x30, #176]",
    "ldp x24, x25, [x30, #192]",
    "ldp x26, x27, [x30, #208]",
    "ldp x28, x29, [x30, #224]",
    "ldr x30, [x30, #240]",
    ".globl _axhw_entry_end",
    "_axhw_entry_end:",
);

extern "C" {
    static axhw_entry_start: u8;
    static axhw_entry_end: u8;
    /// libSystem: make freshly written instructions visible to instruction fetch.
    fn sys_icache_invalidate(start: *mut libc::c_void, len: libc::size_t);
}

const BRK: u32 = 0xd420_0000 | (0x7a7 << 5); // brk #0x7a7
const JIT_SIZE: usize = 0x8000; // two 16K pages: stub at the end of the first, program in the second
const DATA_SIZE: usize = 0x10000;

#[repr(C, align(16))]
struct Block([u8; B_SIZE]);

struct Captured {
    signal: i32,
    pc: u64,
    state: State,
}

static mut BLOCK: Block = Block([0; B_SIZE]);
static mut CAPTURED: Captured = Captured { signal: 0, pc: 0, state: State { x: [0; 31], sp: 0, nzcv: 0, q: [[0; 16]; 32] } };

fn rd(off: usize) -> u64 {
    unsafe {
        let b = &*ptr::addr_of!(BLOCK);
        u64::from_le_bytes(b.0[off..off + 8].try_into().unwrap())
    }
}

fn wr(off: usize, v: u64) {
    unsafe {
        let b = &mut *ptr::addr_of_mut!(BLOCK);
        b.0[off..off + 8].copy_from_slice(&v.to_le_bytes())
    }
}

extern "C" fn on_signal(sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    unsafe {
        let uc = ctx as *mut libc::ucontext_t;
        let mc = (*uc).uc_mcontext;
        let ss = &mut (*mc).__ss;
        let ns = &mut (*mc).__ns;

        let cap = &mut *ptr::addr_of_mut!(CAPTURED);
        cap.signal = sig;
        cap.pc = ss.__pc;
        cap.state.x[..29].copy_from_slice(&ss.__x);
        cap.state.x[29] = ss.__fp;
        cap.state.x[30] = ss.__lr;
        cap.state.sp = ss.__sp;
        cap.state.nzcv = (ss.__cpsr as u64) & 0xf000_0000;
        for i in 0..32 {
            cap.state.q[i] = ns.__v[i].to_le_bytes();
        }

        // Resume the host right after its call into the stub, with its saved registers.
        for i in 0..11 {
            ss.__x[18 + i] = rd(B_HX18 + 8 * i);
        }
        ss.__fp = rd(B_HX18 + 8 * 11);
        ss.__lr = rd(B_HX30);
        ss.__pc = rd(B_HX30);
        ss.__sp = rd(B_HSP);
        ss.__x[0] = 0;
        for i in 0..8 {
            ns.__v[8 + i] = rd(B_HD8 + 8 * i) as u128;
        }
        ns.__fpcr = rd(B_HFPCR) as u32;
    }
}

pub struct Machine {
    data: *mut u8,
    jit: *mut u8,
    entry: extern "C" fn(*mut u8) -> u64,
    stub_len: usize,
}

impl Machine {
    pub fn new() -> Result<Machine, String> {
        unsafe {
            // Ask for low, fixed-looking addresses so corpora from different runs look alike.
            // They are only hints; whatever the kernel picks is recorded in the header.
            let data = libc::mmap(
                0x2_0000_0000 as *mut libc::c_void,
                DATA_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            );
            if data == libc::MAP_FAILED {
                return Err(format!("mmap data: {}", std::io::Error::last_os_error()));
            }
            let jit = libc::mmap(
                0x2_0010_0000 as *mut libc::c_void,
                JIT_SIZE,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_JIT,
                -1,
                0,
            );
            if jit == libc::MAP_FAILED {
                return Err(format!("mmap MAP_JIT: {}", std::io::Error::last_os_error()));
            }

            let start = ptr::addr_of!(axhw_entry_start);
            let stub_len = ptr::addr_of!(axhw_entry_end) as usize - start as usize;
            let stub_at = (jit as *mut u8).add(JIT_SIZE / 2 - stub_len);
            libc::pthread_jit_write_protect_np(0);
            ptr::copy_nonoverlapping(start, stub_at, stub_len);
            libc::pthread_jit_write_protect_np(1);
            sys_icache_invalidate(stub_at as *mut libc::c_void, stub_len);

            // The guest sp is random, so signals must run on their own stack.
            let alt_size = 1 << 16;
            let alt = libc::mmap(ptr::null_mut(), alt_size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0);
            if alt == libc::MAP_FAILED {
                return Err("mmap signal stack".into());
            }
            let ss = libc::stack_t { ss_sp: alt, ss_size: alt_size, ss_flags: 0 };
            if libc::sigaltstack(&ss, ptr::null_mut()) != 0 {
                return Err("sigaltstack".into());
            }
            for sig in [libc::SIGTRAP, libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE] {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = on_signal as *const () as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut sa.sa_mask);
                if libc::sigaction(sig, &sa, ptr::null_mut()) != 0 {
                    return Err(format!("sigaction {sig}"));
                }
            }

            Ok(Machine {
                data: data as *mut u8,
                jit: jit as *mut u8,
                entry: std::mem::transmute::<*mut u8, extern "C" fn(*mut u8) -> u64>(stub_at),
                stub_len,
            })
        }
    }

    pub fn header(&self, host: String) -> Header {
        Header { data: self.data as u64, code: self.code() as u64, host }
    }

    fn code(&self) -> *mut u8 {
        unsafe { self.jit.add(JIT_SIZE / 2) }
    }

    /// Runs one program once. `Err((signal, pc))` if it faulted before reaching the end.
    pub fn run(&mut self, words: &[u32], st: &State, win: &[u8]) -> Result<(State, Vec<u8>), (u32, u64)> {
        assert_eq!(win.len(), WIN_LEN);
        let window = (WIN_ADDR - DATA) as usize;
        let code = self.code();
        unsafe {
            ptr::write_bytes(self.data, 0, DATA_SIZE);
            ptr::copy_nonoverlapping(win.as_ptr(), self.data.add(window), WIN_LEN);

            libc::pthread_jit_write_protect_np(0);
            ptr::write_bytes(code, 0, JIT_SIZE / 2);
            let c = code as *mut u32;
            for (i, w) in words.iter().enumerate() {
                c.add(i).write(*w);
            }
            c.add(words.len()).write(BRK);
            libc::pthread_jit_write_protect_np(1);
            sys_icache_invalidate(code as *mut libc::c_void, 4 * (words.len() + 1));

            for i in 0..31 {
                wr(B_X + 8 * i, st.x[i]);
            }
            wr(B_SP, st.sp);
            wr(B_NZCV, st.nzcv);
            let block = &mut *ptr::addr_of_mut!(BLOCK);
            for i in 0..32 {
                block.0[B_Q + 16 * i..B_Q + 16 * i + 16].copy_from_slice(&st.q[i]);
            }

            (*ptr::addr_of_mut!(CAPTURED)).signal = 0;
            (self.entry)(ptr::addr_of_mut!(BLOCK) as *mut u8);

            let cap = &*ptr::addr_of!(CAPTURED);
            let brk_pc = code as u64 + 4 * words.len() as u64;
            if cap.signal != libc::SIGTRAP || cap.pc != brk_pc {
                return Err((cap.signal as u32, cap.pc));
            }
            let mut out = vec![0u8; WIN_LEN];
            ptr::copy_nonoverlapping(self.data.add(window), out.as_mut_ptr(), WIN_LEN);
            Ok((cap.state.clone(), out))
        }
    }

    /// Runs a case twice and classifies it.
    pub fn record(&mut self, words: Vec<u32>, state: State, win: Vec<u8>) -> Case {
        let a = self.run(&words, &state, &win);
        let b = self.run(&words, &state, &win);
        let outcome = match (a, b) {
            (Ok(x), Ok(y)) if x == y => Outcome::Ran { state: x.0, win: x.1 },
            (Err((signal, pc)), Err(e2)) if (signal, pc) == e2 => Outcome::Faulted { signal, pc: pc.wrapping_sub(self.code() as u64) },
            _ => Outcome::NotDeterministic,
        };
        Case { words, state, win, outcome }
    }

    pub fn stub_len(&self) -> usize {
        self.stub_len
    }
}
