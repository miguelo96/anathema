use anathema_xlat::decode::*;
use anathema_xlat::trans::*;
use std::collections::HashMap;
use std::time::Instant;

struct Img<'a> {
    base: u64,
    data: &'a [u8],
}

impl<'a> Fetch for Img<'a> {
    fn word(&self, pc: u64) -> Option<u32> {
        let o = pc.wrapping_sub(self.base);
        if o >= self.data.len() as u64 || o & 3 != 0 {
            return None;
        }
        let o = o as usize;
        Some(u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap()))
    }
}

struct Stats {
    insns: u64,
    translated: u64,
    hot: u64,
    cold: u64,
    meta: u64,
    mems: u64,
    traces: u64,
    refused: u64,
    hooked: u64,
    unsup: HashMap<String, u64>,
    hooks: HashMap<String, u64>,
    nanos: u128,
    sizes: Vec<(u32, u32)>,
}

fn sweep(cfg: &Config, img: &Img, collect_sizes: bool) -> Stats {
    let mut st = Stats {
        insns: 0,
        translated: 0,
        hot: 0,
        cold: 0,
        meta: 0,
        mems: 0,
        traces: 0,
        refused: 0,
        hooked: 0,
        unsup: HashMap::new(),
        hooks: HashMap::new(),
        nanos: 0,
        sizes: Vec::new(),
    };
    let end = img.base + img.data.len() as u64;
    let mut pc = img.base;
    let t0 = Instant::now();
    while pc < end {
        let t = translate(cfg, img, pc);
        if t.n_insns == 0 {
            st.insns += 1;
            if let Some((w, n)) = t.refused {
                st.refused += 1;
                let key = if n.starts_with("simd") || n == "sve" || n == "sme" {
                    format!("{} {:03x}/{:02x}", n, w >> 21, (w >> 10) & 0x3f)
                } else {
                    n.to_string()
                };
                *st.unsup.entry(key).or_insert(0) += 1;
            } else {
                st.hooked += 1;
                let w = img.word(pc).unwrap();
                *st.hooks.entry(decode(w).name().to_string()).or_insert(0) += 1;
            }
            pc += 4;
            continue;
        }
        st.insns += t.n_insns as u64;
        st.translated += t.n_insns as u64;
        st.hot += t.hot.len() as u64;
        st.cold += t.cold.len() as u64;
        st.meta += t.meta.len() as u64 * 6;
        st.mems += t.n_mem as u64;
        st.traces += 1;
        if collect_sizes {
            st.sizes.push((t.n_insns as u32, t.total() as u32));
        }
        pc = t.end_pc;
    }
    st.nanos = t0.elapsed().as_nanos();
    st
}

fn report(name: &str, st: &Stats) {
    let tr = st.translated.max(1) as f64;
    println!(
        "[{}] insns {} translated {} ({:.1}%) traces {} avg trace {:.1}",
        name,
        st.insns,
        st.translated,
        100.0 * st.translated as f64 / st.insns as f64,
        st.traces,
        tr / st.traces.max(1) as f64
    );
    println!(
        "[{}] hot {} cold {} meta {} total {} bytes: {:.2} B/insn hot, {:.2} B/insn hot+cold, {:.2} B/insn incl meta",
        name,
        st.hot,
        st.cold,
        st.meta,
        st.hot + st.cold,
        st.hot as f64 / tr,
        (st.hot + st.cold) as f64 / tr,
        (st.hot + st.cold + st.meta) as f64 / tr
    );
    println!(
        "[{}] compile {:.1} ms, {:.1} ns/insn, refused {} hooked {} mem insns {}",
        name,
        st.nanos as f64 / 1e6,
        st.nanos as f64 / st.insns as f64,
        st.refused,
        st.hooked,
        st.mems
    );
    if !st.sizes.is_empty() {
        let mut r: Vec<f64> = st.sizes.iter().map(|&(n, b)| b as f64 / n as f64).collect();
        r.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "[{}] per-trace B/insn p10 {:.1} p50 {:.1} p90 {:.1} p99 {:.1}",
            name,
            r[r.len() / 10],
            r[r.len() / 2],
            r[r.len() * 9 / 10],
            r[r.len() * 99 / 100]
        );
    }
}

fn top(m: &HashMap<String, u64>, n: usize, total: u64) {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (k, c) in v.into_iter().take(n) {
        println!("  {:>9} {:5.2}%  {}", c, 100.0 * *c as f64 / total as f64, k);
    }
}

fn stela_compare(cfg: &Config, path: &str) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut their = 0u64;
    let mut ours = 0u64;
    let mut ins = 0u64;
    let mut blocks = 0u64;
    let mut skipped = 0u64;
    for line in text.lines() {
        let pc = match field(line, "\"pc\":") {
            Some(v) => v.parse::<u64>().unwrap_or(0),
            None => continue,
        };
        let src = match line.find("\"source\":\"") {
            Some(p) => {
                let r = &line[p + 10..];
                &r[..r.find('"').unwrap()]
            }
            None => continue,
        };
        let nat = match line.find("\"native\":\"") {
            Some(p) => {
                let r = &line[p + 10..];
                r[..r.find('"').unwrap()].len() / 2
            }
            None => continue,
        };
        let bytes: Vec<u8> = (0..src.len() / 2).map(|i| u8::from_str_radix(&src[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let img = Img { base: pc, data: &bytes };
        let t = translate(cfg, &img, pc);
        if t.n_insns * 4 != bytes.len() {
            skipped += 1;
            continue;
        }
        blocks += 1;
        ins += t.n_insns as u64;
        ours += t.total() as u64;
        their += nat as u64;
    }
    println!(
        "[stela blocks] {} blocks ({} skipped as not fully translatable) {} insns: stela {:.2} B/insn, ours {:.2} B/insn ({:.2}x smaller)",
        blocks,
        skipped,
        ins,
        their as f64 / ins as f64,
        ours as f64 / ins as f64,
        their as f64 / ours.max(1) as f64
    );
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let p = line.find(key)? + key.len();
    let r = &line[p..];
    let e = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
    Some(&r[..e])
}

fn access_costs(base: u64, order: &[u8]) {
    let samples: [(&str, u32); 8] = [
        ("ldr x0,[x1,#8]", 0xf9400420),
        ("str x0,[x1,#8]", 0xf9000420),
        ("ldr w0,[x1,x2,lsl #2]", 0xb8627820),
        ("ldp x29,x30,[sp,#16]", 0xa9417bfd),
        ("stp x29,x30,[sp,#-16]!", 0xa9bf7bfd),
        ("ldrb w0,[x1],#1", 0x38401420),
        ("ldr q0,[x1,#16]", 0x3dc00420),
        ("ldar x0,[x1]", 0xc8dffc20),
    ];
    for (name, model) in [("identity", MemModel::Identity), ("biased", MemModel::Biased), ("checked", MemModel::Checked)] {
        let mut cfg = Config::with_order(model, order);
        cfg.pc_base = base;
        let mut out = String::new();
        for (n, w) in samples {
            let words = [w, 0xd65f03c0u32];
            let bytes: Vec<u8> = words.iter().flat_map(|x| x.to_le_bytes()).collect();
            let img = Img { base, data: &bytes };
            let t = translate(&cfg, &img, base);
            let ret = translate(&cfg, &Img { base, data: &0xd65f03c0u32.to_le_bytes() }, base);
            out += &format!(" [{}: {} B]", n, t.hot.len() as i64 - ret.hot.len() as i64 + 0);
        }
        println!("[{}] hot bytes per access (excl. ret):{}", name, out);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut path = format!("{}/anathema-work/kc_text_exec.bin", std::env::var("HOME").unwrap());
    let mut base: u64 = 0xfffffe0008ae8000;
    let mut stela = format!("{}/anathema-work/s.jsonl", std::env::var("HOME").unwrap());
    let mut sizes = true;
    let mut dump: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                path = args[i + 1].clone();
                i += 1;
            }
            "--base" => {
                base = u64::from_str_radix(args[i + 1].trim_start_matches("0x"), 16).unwrap();
                i += 1;
            }
            "--stela" => {
                stela = args[i + 1].clone();
                i += 1;
            }
            "--nosizes" => sizes = false,
            "--dump-refused" => {
                dump = Some(args[i + 1].clone());
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let data = std::fs::read(&path).expect("read kernel text");
    let img = Img { base, data: &data };
    let n = data.len() / 4;
    println!("input {} bytes, {} insns, base {:#x}", data.len(), n, base);
    let mut uses = [0u64; 33];
    let mut vuses = [0u64; 32];
    let mut classes: HashMap<String, u64> = HashMap::new();
    for k in 0..n {
        let w = u32::from_le_bytes(data[k * 4..k * 4 + 4].try_into().unwrap());
        let i = decode(w);
        i.reg_uses(&mut |r| uses[r as usize] += 1);
        i.vreg_uses(&mut |r| vuses[r as usize] += 1);
        *classes.entry(i.name().to_string()).or_insert(0) += 1;
    }
    if let Some(p) = &dump {
        let mut out = Vec::new();
        for k in 0..n {
            let w = u32::from_le_bytes(data[k * 4..k * 4 + 4].try_into().unwrap());
            if let Insn::Unsup(_) = decode(w) {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        std::fs::write(p, out).unwrap();
    }
    let mut order: Vec<u8> = (0..33u8).filter(|&g| g != 31).collect();
    order.sort_by(|a, b| uses[*b as usize].cmp(&uses[*a as usize]));
    println!("guest reg static uses, top: {:?}", order.iter().take(12).map(|&g| (g, uses[g as usize])).collect::<Vec<_>>());
    access_costs(base, &order);
    let mut vorder: Vec<u8> = (0..32u8).collect();
    vorder.sort_by(|a, b| vuses[*b as usize].cmp(&vuses[*a as usize]));
    println!("guest vreg static uses, top: {:?}", vorder.iter().take(14).map(|&g| (g, vuses[g as usize])).collect::<Vec<_>>());
    println!("decode classes:");
    top(&classes, 14, n as u64);
    for (name, model) in [("identity", MemModel::Identity), ("biased", MemModel::Biased), ("checked", MemModel::Checked)] {
        let mut cfg = Config::with_order(model, &order);
        cfg.set_vorder(&vorder);
        cfg.pc_base = base;
        let st = sweep(&cfg, &img, sizes);
        report(name, &st);
        if model == MemModel::Identity {
            println!("unsupported (refused) opcode histogram:");
            top(&st.unsup, 30, st.insns);
            println!("hooked (exit to runtime) histogram:");
            top(&st.hooks, 12, st.insns);
        }
        if model == MemModel::Identity {
            stela_compare(&cfg, &stela);
        }
    }
    let mut cfg = Config::new(MemModel::Identity);
    cfg.pc_base = base;
    let st = sweep(&cfg, &img, false);
    println!("[identity default pin order] hot+cold {:.2} B/insn", (st.hot + st.cold) as f64 / st.translated as f64);
}
