//! anathema-hwcheck: check the translator against real arm64 hardware.
//!
//! `record` (Apple silicon Mac) generates random arm64 programs with the same generator as the
//! QEMU diff tests, runs each one directly on the CPU and saves inputs + outputs to a corpus.
//! `replay` (x86-64 Linux) runs the same programs through anathema's translator and reports
//! every difference. Real hardware is the reference, so a difference is a translator bug,
//! a decoder bug, or (rarely) a case the generator should not produce.

// Writing corpora only happens on the Mac; replay hosts only read them.
#[cfg_attr(not(all(target_os = "macos", target_arch = "aarch64")), allow(dead_code))]
mod corpus;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;

use corpus::{Case, Outcome};
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

const USAGE: &str = "\
usage:
  anathema-hwcheck record [--cases N] [--seed S] [--max-len L] [--out FILE] [--fp-specials]   (Apple silicon Mac)
  anathema-hwcheck replay FILE [--model identity|biased|checked|kernel] [--show N] [--strict] [--trace] [--only N]   (x86-64 Linux)
  anathema-hwcheck info FILE
  anathema-hwcheck show FILE N   (one case: program, inputs, hardware result)
  anathema-hwcheck selftest   (Apple silicon Mac: check the recording harness itself)";

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> Result<T, String> {
    match args.iter().position(|a| a == name) {
        None => Ok(default),
        Some(i) => {
            let v = args.get(i + 1).ok_or(format!("{name} needs a value"))?;
            let v = v.strip_prefix("0x").map(|h| u64::from_str_radix(h, 16).map(|n| n.to_string())).unwrap_or(Ok(v.clone()));
            v.ok().and_then(|v| v.parse().ok()).ok_or(format!("bad value for {name}"))
        }
    }
}

fn disasm(words: &[u32]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(i, w)| format!("  {:3}: {:08x}  {}\n", i * 4, w, anathema_xlat::decode::decode(*w).name()))
        .collect()
}

fn read_corpus(path: &str) -> Result<(corpus::Header, Vec<Case>), String> {
    let f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut r = BufReader::new(f);
    let h = corpus::read_header(&mut r).map_err(|e| format!("{path}: {e}"))?;
    let mut cases = Vec::new();
    while let Some(c) = corpus::read_case(&mut r).map_err(|e| format!("{path}: case {}: {e}", cases.len()))? {
        cases.push(c);
    }
    Ok((h, cases))
}

fn info(args: &[String]) -> Result<ExitCode, String> {
    let path = args.first().ok_or(USAGE)?;
    let (h, cases) = read_corpus(path)?;
    let ran = cases.iter().filter(|c| matches!(c.outcome, Outcome::Ran { .. })).count();
    let faulted = cases.iter().filter(|c| matches!(c.outcome, Outcome::Faulted { .. })).count();
    println!("host:     {}", h.host);
    println!("data:     {:#x}", h.data);
    println!("code:     {:#x}", h.code);
    println!("cases:    {} ({} ran, {} faulted, {} not deterministic)", cases.len(), ran, faulted, cases.len() - ran - faulted);
    let mut by_sig = std::collections::BTreeMap::new();
    for c in &cases {
        if let Outcome::Faulted { signal, .. } = c.outcome {
            *by_sig.entry(signal).or_insert(0) += 1;
        }
    }
    for (s, n) in by_sig {
        let name = match s {
            4 => " (SIGILL: undefined instruction on this CPU)",
            10 => " (SIGBUS: usually alignment)",
            11 => " (SIGSEGV)",
            _ => "",
        };
        println!("  signal {s}: {n}{name}");
    }
    // The instructions the CPU refused are the interesting ones: if the decoder accepts them,
    // the translator runs code that real hardware treats as undefined.
    for c in &cases {
        if let Outcome::Faulted { signal: 4, pc } = c.outcome {
            if let Some(w) = c.words.get((pc / 4) as usize) {
                println!("  SIGILL at +{pc:#x}: {w:08x}  decoder says {}", anathema_xlat::decode::decode(*w).name());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints one case: program, input registers, and what the hardware did.
fn show_case(args: &[String]) -> Result<ExitCode, String> {
    let (path, n) = match args {
        [p, n, ..] => (p, n.parse::<usize>().map_err(|_| USAGE.to_string())?),
        _ => return Err(USAGE.into()),
    };
    let (h, cases) = read_corpus(path)?;
    let c = cases.get(n).ok_or(format!("only {} cases", cases.len()))?;
    println!("case {n}  (data {:#x}, code {:#x})\n{}", h.data, h.code, disasm(&c.words));
    let regs = |s: &anathema_xlat::oracle::State| {
        let mut out = String::new();
        for (i, x) in s.x.iter().enumerate() {
            out += &format!("x{i:<2} {x:#018x}{}", if i % 4 == 3 { "\n" } else { "  " });
        }
        out += &format!("sp  {:#018x}  nzcv {:x}\n", s.sp, s.nzcv >> 28);
        out
    };
    println!("input:\n{}", regs(&c.state));
    match &c.outcome {
        Outcome::Ran { state, .. } => println!("hardware ran it, output:\n{}", regs(state)),
        Outcome::Faulted { signal, pc } => println!("hardware: signal {signal} at +{pc:#x}"),
        Outcome::NotDeterministic => println!("hardware: not deterministic"),
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn record(args: &[String]) -> Result<ExitCode, String> {
    use anathema_xlat::oracle::{gen_program_opts, gen_state_opts, GenOpts, Rng, BASEREG, DATA, MAXW};
    use std::io::{BufWriter, Write};
    let cases: usize = arg(args, "--cases", 2000)?;
    let seed: u64 = arg(args, "--seed", 0x5eed_0001)?;
    let max_len: usize = arg(args, "--max-len", 16)?;
    let out: String = arg(args, "--out", "hw-corpus.bin".to_string())?;
    // The stock generator never produces NaN or infinity inputs (it clears the top exponent
    // bit). --fp-specials seeds vector registers with ±0, ±inf, quiet and signalling NaNs.
    let fp_specials = args.iter().any(|a| a == "--fp-specials");
    if max_len == 0 || max_len > MAXW {
        return Err(format!("--max-len must be 1..={MAXW}"));
    }

    let mut m = native::Machine::new()?;
    let host = host_description();

    // x18 is the platform register on Darwin. Check the kernel leaves it alone across our
    // entry stub and the signal path, otherwise programs that use it can't be compared.
    {
        let mut st = anathema_xlat::oracle::State { x: [0; 31], sp: 0, nzcv: 0, q: [[0; 16]; 32] };
        st.x[18] = 0x1818_1818_1818_1818;
        st.x[28] = m.header(String::new()).data + (BASEREG - DATA);
        let win = vec![0u8; anathema_xlat::oracle::WIN_LEN];
        let nop = 0xd503_201f;
        match m.run(&[nop], &st, &win) {
            Ok((s, _)) if s.x[18] == st.x[18] => {}
            Ok((s, _)) => return Err(format!("x18 not preserved ({:#x}): this host can't record a reliable corpus", s.x[18])),
            Err((sig, pc)) => return Err(format!("self-check faulted: signal {sig} at {pc:#x}")),
        }
    }

    let header = m.header(if fp_specials { format!("{host} [fp-specials]") } else { host });
    let f = File::create(&out).map_err(|e| format!("{out}: {e}"))?;
    let mut w = BufWriter::new(f);
    corpus::write_header(&mut w, &header).map_err(|e| e.to_string())?;

    let base = header.data + (BASEREG - DATA);
    let mut rng = Rng(seed);
    let mut special_rng = Rng(seed ^ 0x5bec_1a15);
    const SPECIAL_D: [u64; 12] = [
        0, 1 << 63, 0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000, // ±0, ±inf
        0x7ff8_0000_0000_0000, 0xfff8_0000_0000_0000, 0x7ff8_0000_dead_beef, 0xfffc_0000_0000_1234, // quiet NaNs
        0x7ff0_0000_0000_0001, 0xfff4_0000_0000_0bad, // signalling NaNs
        0x3ff0_0000_0000_0000, 0xbff0_0000_0000_0000, // ±1.0
    ];
    const SPECIAL_S: [u32; 12] = [
        0, 1 << 31, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0xffc0_0000, 0x7fc0_beef, 0xffe0_1234, 0x7f80_0001, 0xffa0_0bad,
        0x3f80_0000, 0xbf80_0000,
    ];
    let (mut ran, mut faulted, mut nondet) = (0, 0, 0);
    for i in 0..cases {
        let opts = GenOpts { sp_base: i % 2 == 1 };
        let n = 1 + rng.below(max_len as u64) as usize;
        let words = gen_program_opts(&mut rng, n, &opts);
        let (mut st, win) = gen_state_opts(&mut rng, &opts);
        if fp_specials {
            for q in st.q.iter_mut() {
                match special_rng.below(3) {
                    0 => q[..8].copy_from_slice(&SPECIAL_D[special_rng.below(12) as usize].to_le_bytes()),
                    1 => q[..4].copy_from_slice(&SPECIAL_S[special_rng.below(12) as usize].to_le_bytes()),
                    _ => {}
                }
            }
        }
        st.x[28] = base;
        if opts.sp_base {
            st.sp = base;
        }
        let case = m.record(words, st, win);
        match case.outcome {
            Outcome::Ran { .. } => ran += 1,
            Outcome::Faulted { .. } => faulted += 1,
            Outcome::NotDeterministic => nondet += 1,
        }
        corpus::write_case(&mut w, &case).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    println!("{}", header.host);
    println!("data {:#x}  code {:#x}  (entry stub {} bytes)", header.data, header.code, m.stub_len());
    println!("wrote {out}: {cases} cases ({ran} ran, {faulted} faulted, {nondet} not deterministic), seed {seed:#x}");
    Ok(ExitCode::SUCCESS)
}

/// Known programs with known answers, to check the harness itself before trusting a corpus.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn selftest() -> Result<ExitCode, String> {
    use anathema_xlat::oracle::{State, BASEREG, DATA, WIN_LEN};
    let mut m = native::Machine::new()?;
    let h = m.header(String::new());
    let base = h.data + (BASEREG - DATA);
    let blank = || {
        let mut s = State { x: [0; 31], sp: 0, nzcv: 0, q: [[0; 16]; 32] };
        s.x[28] = base;
        s
    };
    let win = vec![0u8; WIN_LEN];
    let mut fails = 0;
    let mut check = |name: &str, ok: bool| {
        println!("{} {name}", if ok { "ok  " } else { "FAIL" });
        fails += !ok as usize;
    };

    let mut st = blank();
    st.x[1] = 41;
    let r = m.run(&[0x9100_0420], &st, &win); // add x0, x1, #1
    check("add x0, x1, #1", matches!(&r, Ok((s, _)) if s.x[0] == 42 && s.x[1] == 41));

    let mut st = blank();
    st.x[1] = u64::MAX;
    st.x[2] = 1;
    let r = m.run(&[0xab02_0020], &st, &win); // adds x0, x1, x2
    check("adds x0, x1, x2 sets Z and C", matches!(&r, Ok((s, _)) if s.x[0] == 0 && s.nzcv == 0x6000_0000));

    let mut st = blank();
    st.x[1] = 0x1122_3344_5566_7788;
    let r = m.run(&[0xf900_0381], &st, &win); // str x1, [x28]
    check("str x1, [x28] lands in the window", matches!(&r, Ok((_, w)) if w[512..520] == 0x1122_3344_5566_7788u64.to_le_bytes()));

    let mut w2 = win.clone();
    w2[520..528].copy_from_slice(&0xdead_beefu64.to_le_bytes());
    let r = m.run(&[0xf940_0780], &blank(), &w2); // ldr x0, [x28, #8]
    check("ldr x0, [x28, #8] reads the window", matches!(&r, Ok((s, _)) if s.x[0] == 0xdead_beef));

    let r = m.run(&[0x1000_0000], &blank(), &win); // adr x0, #0
    check("adr x0, #0 gives the recorded code address", matches!(&r, Ok((s, _)) if s.x[0] == h.code));

    let mut st = blank();
    st.q[1][..8].copy_from_slice(&1.0f64.to_bits().to_le_bytes());
    st.q[2][..8].copy_from_slice(&2.0f64.to_bits().to_le_bytes());
    let r = m.run(&[0x1e62_2820], &st, &win); // fadd d0, d1, d2
    check("fadd d0, d1, d2", matches!(&r, Ok((s, _)) if s.q[0][..8] == 3.0f64.to_bits().to_le_bytes()));

    let mut st = blank();
    st.sp = 0x1234_5678_9abc_def1; // random, unaligned: must survive untouched
    st.x[30] = 0x3030;
    st.nzcv = 0x9000_0000;
    let r = m.run(&[0xd503_201f], &st, &win); // nop
    check("sp, x30, nzcv pass through", matches!(&r, Ok((s, _)) if s.sp == st.sp && s.x[30] == 0x3030 && s.nzcv == 0x9000_0000));

    let r = m.run(&[0xd503_201f, 0x0000_0000], &blank(), &win); // nop; udf #0
    check("udf #0 reported as SIGILL at +4", r == Err((libc::SIGILL as u32, h.code + 4)));

    let r = m.run(&[0x9100_0420], &st, &win); // harness still sane after a fault
    check("runs again after a fault", r.is_ok());

    println!("{}", if fails == 0 { "harness ok" } else { "harness BROKEN: don't trust corpora from this host" });
    Ok(if fails == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn selftest() -> Result<ExitCode, String> {
    Err("selftest needs an Apple silicon Mac".into())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn host_description() -> String {
    let sysctl = |name: &str| {
        std::process::Command::new("sysctl")
            .args(["-n", name])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    format!("{} / macOS {} ({})", sysctl("machdep.cpu.brand_string"), sysctl("kern.osproductversion"), sysctl("kern.osversion"))
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn record(_: &[String]) -> Result<ExitCode, String> {
    Err("record runs arm64 code directly, so it needs an Apple silicon Mac".into())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn replay(args: &[String]) -> Result<ExitCode, String> {
    use anathema_xlat::oracle::{diff, Native};
    use anathema_xlat::trans::{Config, MemModel};
    let path = args.first().filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
    let model: String = arg(args, "--model", "identity".to_string())?;
    let show: usize = arg(args, "--show", 10)?;
    let strict = args.iter().any(|a| a == "--strict");
    // Print each case number to stderr before running it, to find a case that crashes the
    // translator itself. `--only N` replays a single case.
    let trace = args.iter().any(|a| a == "--trace");
    let only: Option<usize> = args.iter().position(|a| a == "--only").map(|_| arg(args, "--only", 0)).transpose()?;
    let cfg = match model.as_str() {
        "identity" => Config::new(MemModel::Identity),
        "biased" => {
            let mut c = Config::new(MemModel::Biased);
            c.bias = 0x1000_0000;
            c
        }
        "checked" => Config::new(MemModel::Checked),
        "kernel" => {
            // Register pinning used by the runtime for the kernel (see tests/diff.rs).
            let mut c = Config::with_order(MemModel::Identity, &[8, 0, 32, 16, 9, 30, 17, 19, 1, 20, 10]);
            c.set_vorder(&[0, 1, 2, 3, 8, 4, 9, 5, 6, 7, 16]);
            c
        }
        m => return Err(format!("unknown model {m}")),
    };

    let (h, cases) = read_corpus(path)?;
    println!("corpus: {} cases recorded on {}", cases.len(), h.host);
    let mut nat = Native::at(cfg, h.data, h.code);
    let (mut same, mut mismatch, mut nan_only, mut errors, mut skipped, mut accepts_illegal, mut pacga) = (0, 0, 0, 0, 0, 0, 0);
    let mut illegal_by_kind = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut shown = 0;
    for (i, c) in cases.iter().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        if trace {
            eprintln!("case {i}");
        }
        match &c.outcome {
            Outcome::Ran { state, win } => match nat.run(&c.state, &c.win, &c.words) {
                Ok((s, w)) => {
                    let exact = s == *state && w == *win;
                    match diff(state, win, &s, &w) {
                        None if exact => same += 1,
                        // PACGA hashes with the CPU's secret per-process key, so hardware and
                        // translator can never agree on it (or on anything computed from it).
                        _ if uses_pacga(&c.words) => pacga += 1,
                        None if !strict => nan_only += 1,
                        d => {
                            mismatch += 1;
                            if shown < show {
                                shown += 1;
                                let d = d.unwrap_or_else(|| nan_detail(state, &s));
                                println!("\ncase {i}: MISMATCH (hardware vs translator)\n{}{}", disasm(&c.words), d.replace("oracle", "hw").replace("native", "xlat"));
                            }
                        }
                    }
                }
                Err(e) => {
                    errors += 1;
                    if shown < show {
                        shown += 1;
                        println!("\ncase {i}: translator error: {e}\n{}", disasm(&c.words));
                    }
                }
            },
            Outcome::Faulted { signal, pc } => {
                // SIGILL on hardware, but the decoder accepts the instruction: the translator
                // would run an encoding the CPU treats as undefined. Only the decoder is
                // asked. Running the rest of the program is unsafe: code after an undefined
                // instruction never ran on hardware, and in the identity memory model it can
                // write to any host address (e.g. through a base register the bad
                // instruction clobbered).
                let accepted = *signal == 4
                    && c.words.get((*pc / 4) as usize).is_some_and(|w| {
                        use anathema_xlat::decode::{decode, ExcKind, Insn};
                        !matches!(decode(*w), Insn::Unsup(_) | Insn::Exc { kind: ExcKind::Udf, .. })
                    });
                if accepted {
                    accepts_illegal += 1;
                    if let Some(w) = c.words.get((*pc / 4) as usize) {
                        *illegal_by_kind.entry(anathema_xlat::decode::decode(*w).name()).or_default() += 1;
                    }
                    if shown < show {
                        shown += 1;
                        println!("\ncase {i}: hardware raised SIGILL at +{pc:#x}, decoder accepts it\n{}", disasm(&c.words));
                    }
                } else {
                    skipped += 1;
                }
            }
            Outcome::NotDeterministic => skipped += 1,
        }
    }
    println!("\nmodel {model}: {same} identical, {mismatch} mismatches, {errors} translator errors, {accepts_illegal} illegal encodings accepted, {skipped} skipped");
    if nan_only > 0 {
        println!("{nan_only} more differ only in NaN lanes (hidden by oracle::diff's NaN masking; pass --strict to count them as mismatches)");
    }
    if pacga > 0 {
        println!("{pacga} differ because they use PACGA (keyed by the CPU's secret key; expected, not compared)");
    }
    if !illegal_by_kind.is_empty() {
        let kinds: Vec<String> = illegal_by_kind.iter().map(|(k, n)| format!("{k} {n}")).collect();
        println!("illegal encodings accepted, by what the decoder called them: {}", kinds.join(", "));
    }
    Ok(if mismatch + errors + accepts_illegal == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn uses_pacga(words: &[u32]) -> bool {
    use anathema_xlat::decode::{decode, Insn};
    words.iter().any(|w| matches!(decode(*w), Insn::Pac { zero: Some(_) }))
}

/// For differences that oracle::diff masks: show the vector registers bit for bit.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn nan_detail(hw: &anathema_xlat::oracle::State, xlat: &anathema_xlat::oracle::State) -> String {
    let mut s = String::from("NaN lanes differ (hidden by the default NaN masking):\n");
    for i in 0..32 {
        if hw.q[i] != xlat.q[i] {
            let lanes = |q: &[u8; 16]| {
                let lo = u64::from_le_bytes(q[..8].try_into().unwrap());
                let hi = u64::from_le_bytes(q[8..].try_into().unwrap());
                format!("{hi:016x}_{lo:016x}")
            };
            s += &format!("v{i}: hw {} xlat {}\n", lanes(&hw.q[i]), lanes(&xlat.q[i]));
        }
    }
    s
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn replay(_: &[String]) -> Result<ExitCode, String> {
    Err("replay runs translated x86-64 code, so it needs an x86-64 Linux host".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let r = match args.first().map(String::as_str) {
        Some("record") => record(rest),
        Some("replay") => replay(rest),
        Some("info") => info(rest),
        Some("selftest") => selftest(),
        Some("show") => show_case(rest),
        _ => Err(USAGE.to_string()),
    };
    r.unwrap_or_else(|e| {
        eprintln!("{e}");
        ExitCode::from(2)
    })
}
