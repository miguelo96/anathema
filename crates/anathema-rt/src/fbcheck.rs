use crate::simd;
use anathema_xlat::oracle::{gen_state, Oracle, Rng};
use anathema_xlat::run::Ctx;

struct T {
    name: &'static str,
    base: u32,
    rand: u32,
}

fn templates() -> Vec<T> {
    vec![
        T { name: "eor3", base: 0xce00_0000, rand: 0x001f_7fff },
        T { name: "bcax", base: 0xce20_0000, rand: 0x001f_7fff },
        T { name: "rax1", base: 0xce60_8c00, rand: 0x001f_03ff },
        T { name: "xar", base: 0xce80_0000, rand: 0x001f_fbff },
        T { name: "sha512h", base: 0xce60_8000, rand: 0x001f_03ff },
        T { name: "sha512h2", base: 0xce60_8400, rand: 0x001f_03ff },
        T { name: "sha512su1", base: 0xce60_8800, rand: 0x001f_03ff },
        T { name: "sha512su0", base: 0xcec0_8000, rand: 0x0000_03ff },
        T { name: "three-diff", base: 0x0e20_0000, rand: 0x60df_f3ff },
        T { name: "across", base: 0x0e30_0800, rand: 0x40c1_f3ff },
        T { name: "three-same", base: 0x0e20_0400, rand: 0x60df_f8ff },
        T { name: "two-reg", base: 0x0e20_0800, rand: 0x60c1_f3ff },
        T { name: "ext", base: 0x2e00_0000, rand: 0x401f_7fff },
        T { name: "perm", base: 0x0e00_0800, rand: 0x40df_73ff },
        T { name: "tbl", base: 0x0e00_0000, rand: 0x401f_73ff },
        T { name: "shift-imm", base: 0x0f00_0400, rand: 0x607f_f8ff },
        T { name: "mod-imm", base: 0x0f00_0400, rand: 0x6007_f0ff },
        T { name: "vcopy", base: 0x0e00_0400, rand: 0x601f_7bff },
        T { name: "by-elem", base: 0x0f00_0000, rand: 0x60ff_fbff },
        T { name: "scalar2", base: 0x5e20_0800, rand: 0x20c1_f3ff },
        T { name: "sha-3reg", base: 0x5e00_0000, rand: 0x001f_7000 | 0x3ff },
        T { name: "sha-2reg", base: 0x5e28_0800, rand: 0x0000_f000 | 0x3ff },
        T { name: "aes", base: 0x4e28_4800, rand: 0x0000_3000 | 0x3ff },
        T { name: "pmull", base: 0x0e20_e000, rand: 0x40df_03ff },
        T { name: "fabs-fneg", base: 0x0ea0_f800, rand: 0x6040_03ff },
        T { name: "addp-scalar", base: 0x5ef1_b800, rand: 0x3ff },
        T { name: "dup-scalar", base: 0x5e00_0400, rand: 0x001f_03ff },
    ]
}

pub fn run(seed: u64, per: usize) {
    let mut oracle = Oracle::new("/tmp/anx-fbcheck").expect("oracle");
    let mut rng = Rng(seed);
    for t in templates() {
        let mut tested = 0;
        let mut bad = 0;
        let mut shown = 0;
        let mut skipped = 0;
        let mut unhandled = 0;
        for _ in 0..per {
            let r = (rng.next() as u32) & t.rand;
            let w = t.base | r;
            let (st, win) = gen_state(&mut rng);
            let mut ctx = Ctx::default();
            for i in 0..31 {
                ctx.x[i] = st.x[i];
            }
            ctx.q = st.q;
            if !simd::fallback(&mut ctx, w) {
                unhandled += 1;
                continue;
            }
            let o = match oracle.run(&st, &win, &[w]) {
                Ok(o) => o,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            tested += 1;
            let mut diff = String::new();
            for i in 0..32 {
                if o.0.q[i] != ctx.q[i] {
                    diff += &format!(" q{} want {:02x?} got {:02x?}", i, o.0.q[i], ctx.q[i]);
                    break;
                }
            }
            for i in 0..31 {
                if o.0.x[i] != ctx.x[i] {
                    diff += &format!(" x{} want {:#x} got {:#x}", i, o.0.x[i], ctx.x[i]);
                    break;
                }
            }
            if !diff.is_empty() {
                bad += 1;
                if shown < 14 {
                    shown += 1;
                    println!("  MISMATCH {} word {:08x}:{}", t.name, w, &diff[..diff.len().min(30)]);
                }
            }
        }
        println!("{:12} tested {:5} bad {:4} unhandled {:5} oracle-skipped {:5}", t.name, tested, bad, unhandled, skipped);
    }
}
