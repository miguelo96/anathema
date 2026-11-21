use anathema_xlat::run::Ctx;

type V = [u8; 16];

fn rl(q: &V, e: usize, i: usize) -> u64 {
    let mut b = [0u8; 8];
    b[..e].copy_from_slice(&q[i * e..(i + 1) * e]);
    u64::from_le_bytes(b)
}

fn wl(q: &mut V, e: usize, i: usize, v: u64) {
    q[i * e..(i + 1) * e].copy_from_slice(&v.to_le_bytes()[..e]);
}

fn mk(e: usize) -> u64 {
    if e >= 8 {
        u64::MAX
    } else {
        (1u64 << (e * 8)) - 1
    }
}

fn sx(v: u64, e: usize) -> i64 {
    let sh = 64 - e as u32 * 8;
    ((v << sh) as i64) >> sh
}

fn field(w: u32, hi: u32, lo: u32) -> u32 {
    (w >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

pub fn fallback(ctx: &mut Ctx, w: u32) -> bool {
    if w & 0x9f3e_0c00 == 0x0e30_0800 {
        return across(ctx, w);
    }
    if w & 0x9f20_0400 == 0x0e20_0400 {
        return three_same(ctx, w);
    }
    if w & 0x9f3e_0c00 == 0x0e20_0800 {
        return two_reg(ctx, w);
    }
    if w & 0xbfe0_8400 == 0x2e00_0000 {
        return ext(ctx, w);
    }
    if w & 0xbf20_8c00 == 0x0e00_0800 {
        return perm(ctx, w);
    }
    if w & 0xbfe0_9c00 == 0x0e00_0000 {
        return tbl(ctx, w);
    }
    if w & 0x9f80_0400 == 0x0f00_0400 {
        return shift_imm(ctx, w);
    }
    false
}

fn across(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30);
    let u = field(w, 29, 29);
    let size = field(w, 23, 22) as usize;
    let opcode = field(w, 16, 12);
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let e = 1usize << size;
    if e == 8 {
        return false;
    }
    let count = if q == 1 { 16 } else { 8 } / e;
    let m = mk(e);
    let src: Vec<u64> = (0..count).map(|i| rl(&ctx.q[rn], e, i)).collect();
    let (res, osz): (u64, usize) = match (opcode, u) {
        (0x03, 0) => (src.iter().fold(0i64, |a, &v| a.wrapping_add(sx(v, e))) as u64, e * 2),
        (0x03, 1) => (src.iter().fold(0u64, |a, &v| a.wrapping_add(v & m)), e * 2),
        (0x1b, _) => (src.iter().fold(0u64, |a, &v| a.wrapping_add(v)) & m, e),
        (0x0a, 0) => (src.iter().map(|&v| sx(v, e)).max().unwrap() as u64 & m, e),
        (0x0a, 1) => (src.iter().map(|&v| v & m).max().unwrap(), e),
        (0x1a, 0) => (src.iter().map(|&v| sx(v, e)).min().unwrap() as u64 & m, e),
        (0x1a, 1) => (src.iter().map(|&v| v & m).min().unwrap(), e),
        _ => return false,
    };
    let mut out = [0u8; 16];
    out[..osz.min(8)].copy_from_slice(&res.to_le_bytes()[..osz.min(8)]);
    ctx.q[rd] = out;
    true
}

fn three_same(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let u = field(w, 29, 29);
    let size = field(w, 23, 22) as usize;
    let rm = field(w, 20, 16) as usize;
    let opcode = field(w, 15, 11);
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let n = ctx.q[rn];
    let mv = ctx.q[rm];
    let d = ctx.q[rd];
    let total = if q { 16 } else { 8 };
    let mut out = [0u8; 16];
    if opcode == 0x03 {
        for i in 0..total {
            let (a, b, c) = (n[i], mv[i], d[i]);
            out[i] = match (u, size) {
                (0, 0) => a & b,
                (0, 1) => a & !b,
                (0, 2) => a | b,
                (0, _) => a | !b,
                (_, 0) => a ^ b,
                (_, 1) => (c & a) | (!c & b),
                (_, 2) => (c & !b) | (a & b),
                (_, _) => (c & b) | (a & !b),
            };
        }
        ctx.q[rd] = out;
        return true;
    }
    let e = 1usize << size;
    if e == 8 && !matches!(opcode, 0x06 | 0x07 | 0x08 | 0x10 | 0x11 | 0x0c | 0x0d | 0x0e) {
        return false;
    }
    let count = total / e;
    let m = mk(e);
    let pair = matches!(opcode, 0x14 | 0x15 | 0x17);
    let src: Vec<u64> = if pair {
        let mut s: Vec<u64> = (0..count).map(|i| rl(&n, e, i)).collect();
        s.extend((0..count).map(|i| rl(&mv, e, i)));
        s
    } else {
        Vec::new()
    };
    for i in 0..count {
        let a = rl(&n, e, i);
        let b = rl(&mv, e, i);
        let c = rl(&d, e, i);
        let (sa, sb) = (sx(a, e), sx(b, e));
        let r: u64 = match (opcode, u) {
            (0x00, 0) => (((sa as i128 + sb as i128) >> 1) as u64) & m,
            (0x00, _) => (((a as u128 + b as u128) >> 1) as u64) & m,
            (0x04, 0) => (((sa as i128 - sb as i128) >> 1) as u64) & m,
            (0x04, _) => (((a as i128 - b as i128) >> 1) as u64) & m,
            (0x06, 0) => if sa > sb { m } else { 0 },
            (0x06, _) => if a > b { m } else { 0 },
            (0x07, 0) => if sa >= sb { m } else { 0 },
            (0x07, _) => if a >= b { m } else { 0 },
            (0x08, _) => {
                let sh = sb as i8 as i32;
                if sh >= 0 {
                    if sh as usize >= e * 8 { 0 } else { (a << sh) & m }
                } else if u == 0 {
                    ((sa >> (-sh).min(63)) as u64) & m
                } else if (-sh) as usize >= e * 8 {
                    0
                } else {
                    (a >> (-sh)) & m
                }
            }
            (0x0c, 0) => (sa.max(sb) as u64) & m,
            (0x0c, _) => a.max(b),
            (0x0d, 0) => (sa.min(sb) as u64) & m,
            (0x0d, _) => a.min(b),
            (0x0e, 0) => ((sa - sb).unsigned_abs()) & m,
            (0x0e, _) => (if a > b { a - b } else { b - a }) & m,
            (0x0f, 0) => c.wrapping_add((sa - sb).unsigned_abs()) & m,
            (0x0f, _) => c.wrapping_add(if a > b { a - b } else { b - a }) & m,
            (0x10, 0) => a.wrapping_add(b) & m,
            (0x10, _) => a.wrapping_sub(b) & m,
            (0x11, 0) => if a & b != 0 { m } else { 0 },
            (0x11, _) => if a == b { m } else { 0 },
            (0x12, 0) => c.wrapping_add(a.wrapping_mul(b)) & m,
            (0x12, _) => c.wrapping_sub(a.wrapping_mul(b)) & m,
            (0x13, 0) => a.wrapping_mul(b) & m,
            (0x14, 0) => pm(&src, i, count, e, true, true),
            (0x14, _) => pm(&src, i, count, e, true, false),
            (0x15, 0) => pm(&src, i, count, e, false, true),
            (0x15, _) => pm(&src, i, count, e, false, false),
            (0x17, 0) => {
                let (x, y) = src_pair(&src, i, count);
                x.wrapping_add(y) & m
            }
            _ => return false,
        };
        wl(&mut out, e, i, r);
    }
    ctx.q[rd] = out;
    true
}

fn src_pair(src: &[u64], i: usize, count: usize) -> (u64, u64) {
    let _ = count;
    (src[2 * i], src[2 * i + 1])
}

fn pm(src: &[u64], i: usize, count: usize, e: usize, max: bool, signed: bool) -> u64 {
    let _ = count;
    let (x, y) = (src[2 * i], src[2 * i + 1]);
    let m = mk(e);
    if signed {
        let (a, b) = (sx(x, e), sx(y, e));
        (if max { a.max(b) } else { a.min(b) }) as u64 & m
    } else if max {
        x.max(y)
    } else {
        x.min(y)
    }
}

fn two_reg(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let u = field(w, 29, 29);
    let size = field(w, 23, 22) as usize;
    let opcode = field(w, 16, 12);
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let n = ctx.q[rn];
    let total = if q { 16 } else { 8 };
    let e = 1usize << size;
    let mut out = [0u8; 16];
    match (opcode, u) {
        (0x05, 0) if size == 0 => {
            for i in 0..total {
                out[i] = n[i].count_ones() as u8;
            }
        }
        (0x05, 1) if size == 0 => {
            for i in 0..total {
                out[i] = !n[i];
            }
        }
        (0x05, 1) if size == 1 => {
            for i in 0..total {
                out[i] = n[i].reverse_bits();
            }
        }
        (0x00, _) | (0x01, 0) => {
            let container = if opcode == 0x01 { 2 } else if u == 0 { 8 } else { 4 };
            if e >= container {
                return false;
            }
            let per = container / e;
            for c in 0..total / container {
                for j in 0..per {
                    let src = rl(&n, e, c * per + j);
                    wl(&mut out, e, c * per + (per - 1 - j), src);
                }
            }
        }
        (0x04, 1) if e < 8 => {
            let m = mk(e);
            for i in 0..total / e {
                let a = rl(&n, e, i) & m;
                let bits = e as u32 * 8;
                let lz = if a == 0 { bits } else { a.leading_zeros() - (64 - bits) };
                wl(&mut out, e, i, lz as u64);
            }
        }
        (0x08, _) | (0x09, _) | (0x0a, 0) if e <= 8 => {
            let m = mk(e);
            for i in 0..total / e {
                let a = sx(rl(&n, e, i), e);
                let t = match (opcode, u) {
                    (0x08, 0) => a > 0,
                    (0x08, _) => a >= 0,
                    (0x09, 0) => a == 0,
                    (0x09, _) => a <= 0,
                    _ => a < 0,
                };
                wl(&mut out, e, i, if t { m } else { 0 });
            }
        }
        (0x0b, _) if e <= 8 => {
            let m = mk(e);
            for i in 0..total / e {
                let a = sx(rl(&n, e, i), e);
                let r = if u == 0 { a.wrapping_abs() } else { a.wrapping_neg() };
                wl(&mut out, e, i, r as u64 & m);
            }
        }
        (0x12, 0) if e < 8 => {
            let wide = e * 2;
            let half = 8 / e;
            let mut base = if q { ctx.q[rd] } else { [0u8; 16] };
            let lanes = 8 / e;
            for i in 0..lanes {
                let v = rl(&n, wide, i) & mk(e);
                if q {
                    wl(&mut base, e, half + i, v);
                } else {
                    wl(&mut base, e, i, v);
                }
            }
            ctx.q[rd] = base;
            return true;
        }
        _ => return false,
    }
    ctx.q[rd] = out;
    true
}

fn ext(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let rm = field(w, 20, 16) as usize;
    let imm = field(w, 14, 11) as usize;
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let total = if q { 16 } else { 8 };
    let mut cat = [0u8; 32];
    cat[..total].copy_from_slice(&ctx.q[rn][..total]);
    cat[total..total * 2].copy_from_slice(&ctx.q[rm][..total]);
    let mut out = [0u8; 16];
    for i in 0..total {
        out[i] = cat[imm + i];
    }
    ctx.q[rd] = out;
    true
}

fn perm(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let size = field(w, 23, 22) as usize;
    let rm = field(w, 20, 16) as usize;
    let opc = field(w, 14, 12);
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let e = 1usize << size;
    let total = if q { 16 } else { 8 };
    let count = total / e;
    if e == 8 && !q {
        return false;
    }
    let n = ctx.q[rn];
    let m = ctx.q[rm];
    let mut out = [0u8; 16];
    for i in 0..count {
        let v = match opc {
            1 => {
                let j = 2 * i;
                if j < count { rl(&n, e, j) } else { rl(&m, e, j - count) }
            }
            5 => {
                let j = 2 * i + 1;
                if j < count { rl(&n, e, j) } else { rl(&m, e, j - count) }
            }
            2 => if i % 2 == 0 { rl(&n, e, i) } else { rl(&m, e, i - 1) },
            6 => if i % 2 == 0 { rl(&n, e, i + 1) } else { rl(&m, e, i) },
            3 => {
                let j = i / 2;
                if i % 2 == 0 { rl(&n, e, j) } else { rl(&m, e, j) }
            }
            7 => {
                let j = count / 2 + i / 2;
                if i % 2 == 0 { rl(&n, e, j) } else { rl(&m, e, j) }
            }
            _ => return false,
        };
        wl(&mut out, e, i, v);
    }
    ctx.q[rd] = out;
    true
}

fn tbl(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let rm = field(w, 20, 16) as usize;
    let len = field(w, 14, 13) as usize + 1;
    let tbx = field(w, 12, 12) == 1;
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    let total = if q { 16 } else { 8 };
    let idx = ctx.q[rm];
    let old = ctx.q[rd];
    let mut table = Vec::new();
    for k in 0..len {
        table.extend_from_slice(&ctx.q[(rn + k) % 32]);
    }
    let mut out = [0u8; 16];
    for i in 0..total {
        let x = idx[i] as usize;
        out[i] = if x < table.len() { table[x] } else if tbx { old[i] } else { 0 };
    }
    ctx.q[rd] = out;
    true
}

fn shift_imm(ctx: &mut Ctx, w: u32) -> bool {
    let q = field(w, 30, 30) == 1;
    let u = field(w, 29, 29);
    let immh = field(w, 22, 19) as usize;
    let immb = field(w, 18, 16) as usize;
    let opcode = field(w, 15, 11);
    let rn = field(w, 9, 5) as usize;
    let rd = field(w, 4, 0) as usize;
    if immh == 0 {
        return false;
    }
    let k = 31 - (immh as u32).leading_zeros();
    let e = 1usize << k;
    let ebits = e * 8;
    let immhb = (immh << 3) | immb;
    let n = ctx.q[rn];
    let total = if q { 16 } else { 8 };
    let m = mk(e);
    let mut out = [0u8; 16];
    match opcode {
        0x00 | 0x02 => {
            let sh = 2 * ebits - immhb;
            for i in 0..total / e {
                let a = rl(&n, e, i);
                let r = if u == 0 {
                    ((sx(a, e) >> sh.min(63)) as u64) & m
                } else if sh >= 64 {
                    0
                } else {
                    (a >> sh) & m
                };
                let base = if opcode == 0x02 { rl(&ctx.q[rd], e, i) } else { 0 };
                wl(&mut out, e, i, base.wrapping_add(r) & m);
            }
        }
        0x0a => {
            let sh = immhb - ebits;
            for i in 0..total / e {
                let a = rl(&n, e, i);
                let r = if u == 0 {
                    (a << sh) & m
                } else {
                    let keep = !(m << sh) & m;
                    (rl(&ctx.q[rd], e, i) & keep) | ((a << sh) & m)
                };
                wl(&mut out, e, i, r);
            }
        }
        0x14 if e < 8 => {
            let sh = immhb - ebits;
            let wide = e * 2;
            let half = if q { 8 / e } else { 0 };
            for i in 0..8 / e {
                let a = rl(&n, e, half + i);
                let v = if u == 0 { (sx(a, e) << sh) as u64 } else { a << sh };
                wl(&mut out, wide, i, v & mk(wide));
            }
        }
        0x10 if e < 8 => {
            let sh = 2 * ebits - immhb;
            let narrow = e;
            let src = e * 2;
            let half = if q { 8 / narrow } else { 0 };
            let mut base = if q { ctx.q[rd] } else { [0u8; 16] };
            for i in 0..8 / narrow {
                let a = rl(&n, src, i);
                let v = (a >> sh) & mk(narrow);
                wl(&mut base, narrow, half + i, v);
            }
            ctx.q[rd] = base;
            return true;
        }
        _ => return false,
    }
    ctx.q[rd] = out;
    true
}
