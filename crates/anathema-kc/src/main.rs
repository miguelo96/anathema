//! `anathema-kc`: the same command line as tools/kcbuild.py.
//!
//! usage: anathema-kc --kernel KERNEL --ext DIR [--ext DIR2 ...] --out kc.bin --base 0xffffff8002000000 ROOT_ID [ROOT_ID ...]

use anathema_kc::{link, Config};
use anyhow::{anyhow, bail, Context, Result};
use std::path::PathBuf;

fn pair(v: &str, sep: char, flag: &str) -> Result<(String, String)> {
    v.split_once(sep).map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| anyhow!("{flag} wants A{sep}B, got {v}"))
}

/// Python's `int(s, 0)`.
fn int0(s: &str) -> Result<u64> {
    let t = s.trim().replace('_', "");
    let (digits, radix) = match t.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (&t[2..], 16),
        Some("0o") => (&t[2..], 8),
        Some("0b") => (&t[2..], 2),
        _ => (&t[..], 10),
    };
    u64::from_str_radix(digits, radix).with_context(|| format!("bad number {s}"))
}

fn parse(args: &[String]) -> Result<(Config, PathBuf)> {
    let mut c = Config::default();
    let (mut kernel, mut out, mut base) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let Some(flag) = a.strip_prefix("--") else {
            c.roots.push(a.clone());
            i += 1;
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (flag, None),
        };
        let value = match inline {
            Some(v) => v,
            None => {
                i += 1;
                args.get(i).cloned().ok_or_else(|| anyhow!("--{name} needs a value"))?
            }
        };
        match name {
            "kernel" => kernel = Some(PathBuf::from(value)),
            "ext" => c.ext.push(PathBuf::from(value)),
            "override" => {
                let (id, path) = pair(&value, '=', "--override")?;
                c.overrides.push((id, PathBuf::from(path)));
            }
            "out" => out = Some(PathBuf::from(value)),
            "base" => base = Some(int0(&value)?),
            "no-start" => c.no_start.push(value),
            "strip" => c.strip.push(pair(&value, ':', "--strip")?),
            "set-true" => c.set_true.push(pair(&value, ':', "--set-true")?),
            "rename" => c.rename.push(pair(&value, '=', "--rename")?),
            "add-lib" => {
                let mut it = value.splitn(3, ':');
                match (it.next(), it.next(), it.next()) {
                    (Some(a), Some(b), Some(v)) => c.add_lib.push((a.into(), b.into(), v.into())),
                    _ => bail!("--add-lib wants BUNDLE_ID:LIB_ID:VERSION, got {value}"),
                }
            }
            "codeless" => c.codeless.push(value),
            "prefer-last" => c.prefer_last.push(value),
            other => bail!("unknown option --{other}"),
        }
        i += 1;
    }
    c.kernel = kernel.ok_or_else(|| anyhow!("--kernel is required"))?;
    c.base = base.ok_or_else(|| anyhow!("--base is required"))?;
    if c.ext.is_empty() {
        bail!("--ext is required");
    }
    if c.roots.is_empty() {
        bail!("at least one root bundle id is required");
    }
    Ok((c, out.ok_or_else(|| anyhow!("--out is required"))?))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cfg, out) = parse(&args)?;
    let o = link(&cfg)?;
    std::fs::write(&out, &o.xkc).with_context(|| format!("writing {}", out.display()))?;
    let mut map = out.clone().into_os_string();
    map.push(".map");
    std::fs::write(&map, &o.map).with_context(|| format!("writing {}", PathBuf::from(&map).display()))?;
    print!("{}", o.report);
    Ok(())
}
