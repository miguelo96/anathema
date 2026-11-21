use anathema_xlat::decode::*;
use anathema_xlat::oracle::*;
use anathema_xlat::trans::MemModel;
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
    let mut oracle = Oracle::new("/tmp/anx-oracle").expect("oracle");
    let mut native = Native::new(model);
    let mut rng = Rng(seed);
    let mut fails = 0;
    let mut skipped = 0;
    for c in 0..cases {
        let n = 1 + rng.below(len as u64) as usize;
        let prog = gen_program(&mut rng, n);
        let (st, win) = gen_state(&mut rng);
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
