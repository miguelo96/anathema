// Runs translated x86-64 code and compares it with qemu-aarch64, so it only builds on x86-64
// Linux. It also needs `qemu-aarch64` and `aarch64-linux-gnu-objdump` on PATH.
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use anathema_xlat::oracle::*;
use anathema_xlat::trans::{Config, MemModel};
use std::process::Command;

fn dis(words: &[u32]) -> String {
    let mut b = Vec::new();
    for w in words {
        b.extend_from_slice(&w.to_le_bytes());
    }
    let p = "/tmp/anx-dis.bin";
    std::fs::write(p, b).unwrap();
    let o = Command::new("aarch64-linux-gnu-objdump").args(["-D", "-b", "binary", "-m", "aarch64", p]).output().unwrap();
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    s.lines().skip(7).collect::<Vec<_>>().join("\n")
}

fn fuzz(model: MemModel, seed: u64, cases: usize, len: usize) {
    fuzz_cfg(Config::new(model), seed, cases, len, false)
}

fn fuzz_cfg(cfg: Config, seed: u64, cases: usize, len: usize, sp_base: bool) {
    let opts = GenOpts { sp_base };
    let mut oracle = Oracle::new(&format!("/tmp/anx-oracle-{}", seed)).expect("oracle");
    let mut native = Native::with_cfg(cfg);
    let mut rng = Rng(seed);
    let mut fails = 0;
    let mut skipped = 0;
    for c in 0..cases {
        let n = 1 + rng.below(len as u64) as usize;
        let prog = gen_program_opts(&mut rng, n, &opts);
        let (st, win) = gen_state_opts(&mut rng, &opts);
        let o = match oracle.run(&st, &win, &prog) {
            Ok(o) => o,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let nat = match native.run(&st, &win, &prog) {
            Ok(n) => n,
            Err(e) => {
                println!("case {} native error {}\n{}", c, e, dis(&prog));
                fails += 1;
                continue;
            }
        };
        if let Some(d) = diff(&o.0, &o.1, &nat.0, &nat.1) {
            println!("case {} MISMATCH\n{}\n{}", c, dis(&prog), d);
            println!("init: {:x?} sp {:x} nzcv {:x}", st.x, st.sp, st.nzcv >> 28);
            fails += 1;
            if fails > 5 {
                break;
            }
        }
    }
    println!("cases {} fails {} skipped {}", cases, fails, skipped);
    assert_eq!(fails, 0);
}

#[test]
fn identity_short() {
    fuzz(MemModel::Identity, 0x1234_5678_9abc, 3000, 1);
}

#[test]
fn identity_long() {
    fuzz(MemModel::Identity, 0xdead_beef_1234, 3000, 12);
}

#[test]
fn biased() {
    fuzz(MemModel::Biased, 0x77, 500, 8);
}

#[test]
fn checked() {
    fuzz(MemModel::Checked, 0x4242, 500, 8);
}

#[test]
#[ignore]
fn big() {
    fuzz(MemModel::Identity, 0xfeed_f00d_1, 40000, 20);
}

fn kernel_cfg(model: MemModel, bias: u64) -> Config {
    let mut cfg = Config::with_order(model, &[8, 0, 32, 16, 9, 30, 17, 19, 1, 20, 10]);
    cfg.set_vorder(&[0, 1, 2, 3, 8, 4, 9, 5, 6, 7, 16]);
    cfg.bias = bias;
    cfg
}

#[test]
fn kernel_pins_sp_base() {
    fuzz_cfg(kernel_cfg(MemModel::Identity, 0), 0x5151, 4000, 10, true);
}

#[test]
fn kernel_pins_biased_offset() {
    fuzz_cfg(kernel_cfg(MemModel::Biased, 0x1000_0000), 0x6262, 2000, 10, true);
}
