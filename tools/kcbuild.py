#!/usr/bin/env python3
"""link intel kexts into one prelinked block for the anathema loader (no kmutil needed).

usage: kcbuild.py --kernel KERNEL --ext DIR [--ext DIR2 ...] --out kc.bin --base 0xffffff8002000000 ROOT_ID [ROOT_ID ...]
       --override BUNDLE_ID=PATH_TO.kext   (use this bundle instead of the one in --ext)
"""
import argparse, os, struct, sys, plistlib, glob, base64

KBASE = 0xffffff8000000000
MH_DYLIB_IN_CACHE = 0x80000000
PAGE = 0x4000


def macho_slice(d):
    if d[:4] == b"\xca\xfe\xba\xbe":
        n = struct.unpack(">I", d[4:8])[0]
        for i in range(n):
            ct, cs, off, sz, al = struct.unpack(">iiIII", d[8 + 20 * i:28 + 20 * i])
            if ct == 0x01000007:
                return bytearray(d[off:off + sz])
        return None
    if d[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<i", d, 4)[0] == 0x01000007:
        return bytearray(d)
    return None


class Macho:
    def __init__(self, data):
        self.d = data
        magic, ct, cs, ft, nc, sc, fl = struct.unpack_from("<IiiIIII", data, 0)
        self.ncmds, self.flags_off = nc, 24
        self.segs, self.sects = [], []
        self.sym = self.dy = None
        off = 32
        for _ in range(nc):
            c, s = struct.unpack_from("<II", data, off)
            if c == 0x19:
                nm = data[off + 8:off + 24].split(b"\0")[0].decode()
                vm, vs, fo, fs = struct.unpack_from("<QQQQ", data, off + 24)
                ns = struct.unpack_from("<I", data, off + 64)[0]
                self.segs.append(dict(name=nm, vm=vm, vs=vs, fo=fo, fs=fs, lc=off, ns=ns))
                so = off + 72
                for i in range(ns):
                    sn = data[so:so + 16].split(b"\0")[0].decode()
                    addr, size = struct.unpack_from("<QQ", data, so + 32)
                    self.sects.append(dict(seg=nm, name=sn, addr=addr, size=size, lc=so))
                    so += 80
            elif c == 2:
                self.sym = struct.unpack_from("<IIII", data, off + 8)
            elif c == 0xb:
                self.dy = struct.unpack_from("<18I", data, off + 8)
            off += s

    def sym_at(self, i):
        so, ns, sto, sts = self.sym
        n_strx, n_type, n_sect, n_desc, n_value = struct.unpack_from("<IBBHQ", self.d, so + 16 * i)
        e = self.d.index(b"\0", sto + n_strx)
        return self.d[sto + n_strx:e].decode(), n_type, n_desc, n_value

    def exports(self):
        out = {}
        if not self.dy:
            return out
        for i in range(self.dy[2], self.dy[2] + self.dy[3]):
            nm, t, de, v = self.sym_at(i)
            if t & 0x0e == 0x0e:
                out[nm] = v
        return out

    def find_sym(self, name):
        so, ns, sto, sts = self.sym
        for i in range(ns):
            nm, t, de, v = self.sym_at(i)
            if nm == name and (t & 0x0e) == 0x0e:
                return v
        return None


def kernel_symbols(path):
    k = Macho(macho_slice(open(path, "rb").read()))
    so, ns, sto, sts = k.sym
    out = {}
    ext = {}
    for i in range(ns):
        nm, t, de, v = k.sym_at(i)
        if (t & 0x0e) == 0x0e:
            if t & 1:
                ext[nm] = v
            else:
                out.setdefault(nm, v)
    out.update(ext)
    return out


def find_bundles(roots):
    found = {}
    for root in roots:
        for dp, dn, fn in os.walk(root):
            if dp.endswith(".kext") and os.path.exists(os.path.join(dp, "Contents", "Info.plist")):
                p = os.path.join(dp, "Contents", "Info.plist")
            elif dp.endswith(".kext") and os.path.exists(os.path.join(dp, "Info.plist")):
                p = os.path.join(dp, "Info.plist")
            else:
                continue
            try:
                info = plistlib.load(open(p, "rb"))
            except Exception:
                continue
            bid = info.get("CFBundleIdentifier")
            if bid and bid not in found:
                found[bid] = dict(path=dp, plist=p, info=info, root=root)
    return found


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def xml(v, ind=0):
    pad = "\t" * ind
    if isinstance(v, bool):
        return pad + ("<true/>" if v else "<false/>")
    if isinstance(v, int):
        if v < 0:
            v &= 0xffffffffffffffff
        return pad + '<integer size="64">%d</integer>' % v
    if isinstance(v, float):
        return pad + "<real>%r</real>" % v
    if isinstance(v, str):
        return pad + "<string>%s</string>" % esc(v)
    if isinstance(v, (bytes, bytearray)):
        return pad + "<data>%s</data>" % base64.b64encode(bytes(v)).decode()
    if isinstance(v, list):
        return pad + "<array>\n" + "\n".join(xml(x, ind + 1) for x in v) + "\n" + pad + "</array>"
    if isinstance(v, dict):
        out = [pad + "<dict>"]
        for k, x in v.items():
            out.append(pad + "\t<key>%s</key>" % esc(str(k)))
            out.append(xml(x, ind + 1))
        out.append(pad + "</dict>")
        return "\n".join(out)
    if hasattr(v, "isoformat"):
        return pad + "<string>%s</string>" % v.isoformat()
    return pad + "<string>%s</string>" % esc(str(v))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kernel", required=True)
    ap.add_argument("--ext", action="append", required=True)
    ap.add_argument("--override", action="append", default=[])
    ap.add_argument("--out", required=True)
    ap.add_argument("--base", required=True)
    ap.add_argument("--no-start", action="append", default=[], help="bundle ids whose kmod start/stop are zeroed")
    ap.add_argument("--strip", action="append", default=[], help="BUNDLE_ID:KEY removed from that bundle's plist record")
    ap.add_argument("--set-true", action="append", default=[], help="BUNDLE_ID:KEY set to true in that bundle's plist record")
    ap.add_argument("--rename", action="append", default=[], help="OLD_ID=NEW_ID rename in the plist record only")
    ap.add_argument("--add-lib", action="append", default=[], help="BUNDLE_ID:LIB_ID:VERSION added to that record's OSBundleLibraries")
    ap.add_argument("--prefer-last", action="append", default=[], help="bundle ids taken from the last --ext dir when present there")
    ap.add_argument("roots", nargs="+")
    a = ap.parse_args()
    base = int(a.base, 0)

    ksyms = kernel_symbols(a.kernel)
    bundles = find_bundles(a.ext)
    if a.prefer_last:
        last = find_bundles([a.ext[-1]])
        for bid in a.prefer_last:
            if bid in last:
                bundles[bid] = last[bid]
    for o in a.override:
        bid, path = o.split("=", 1)
        ip = os.path.join(path, "Contents", "Info.plist")
        bundles[bid] = dict(path=path, plist=ip, info=plistlib.load(open(ip, "rb")), root=os.path.dirname(path))


    def libs_of(bid):
        inf = bundles[bid]["info"]
        libs = inf.get("OSBundleLibraries_x86_64")
        if libs is None:
            libs = inf.get("OSBundleLibraries")
        out = list(libs or {})
        for al in a.add_lib:
            ab, al_id, al_ver = al.split(":", 2)
            if ab == bid and al_id not in out:
                out.append(al_id)
        return out
    # selection: closure over OSBundleLibraries, plus every pseudo kext (OSKernelResource)
    chosen, order = set(), []

    def want(bid):
        if bid in chosen or bid not in bundles:
            return
        chosen.add(bid)
        for lib in libs_of(bid):
            want(lib)
        order.append(bid)

    for r in a.roots:
        want(r)
    for bid, b in list(bundles.items()):
        if b["info"].get("OSKernelResource") is True:
            want(bid)

    # link
    kcdata = bytearray()
    entries = []
    exports_by = {}
    images = {}
    cur = base
    for bid in order:
        b = bundles[bid]
        exe = b["info"].get("CFBundleExecutable")
        ep = None
        if exe and not b["info"].get("OSKernelResource"):
            for cand in (os.path.join(b["path"], "Contents", "MacOS", exe), os.path.join(b["path"], exe)):
                if os.path.exists(cand):
                    ep = cand
        if not ep:
            entries.append(dict(bid=bid, b=b, exe=None))
            continue
        s = macho_slice(open(ep, "rb").read())
        if s is None:
            print("no x86_64 slice:", bid)
            entries.append(dict(bid=bid, b=b, exe=None))
            continue
        m = Macho(s)
        size = max(sg["vm"] + sg["vs"] for sg in m.segs if sg["name"] != "__LINKEDIT")
        size = (size + PAGE - 1) & ~(PAGE - 1)
        e = dict(bid=bid, b=b, exe=ep, m=m, load=cur, size=size, img=bytearray(size))
        for sg in m.segs:
            if sg["name"] == "__LINKEDIT":
                continue
            e["img"][sg["vm"]:sg["vm"] + sg["fs"]] = s[sg["fo"]:sg["fo"] + sg["fs"]]
        exports_by[bid] = {n: v + cur for n, v in m.exports().items()}
        entries.append(e)
        cur += size

    def deps_closure(bid, seen=None):
        seen = seen if seen is not None else []
        for lib in libs_of(bid):
            if lib in bundles and lib not in seen:
                seen.append(lib)
                deps_closure(lib, seen)
        return seen

    missing = {}
    counts = dict(ext64=0, branch=0, rebase=0)
    for e in entries:
        if not e["exe"]:
            continue
        m, img, load = e["m"], e["img"], e["load"]
        search = [e["bid"]] + deps_closure(e["bid"])
        dy = m.dy
        cache = {}

        def resolve(i):
            nm, t, de, v = m.sym_at(i)
            if nm in cache:
                return cache[nm]
            addr = None
            for bid in search:
                ex = exports_by.get(bid)
                if ex and nm in ex:
                    addr = ex[nm]
                    break
            if addr is None:
                addr = ksyms.get(nm)
            if addr is None:
                for ex in exports_by.values():
                    if nm in ex:
                        addr = ex[nm]
                        break
            if addr is None:
                missing.setdefault(nm, set()).add(e["bid"])
                addr = 0
            cache[nm] = addr
            return addr

        extoff, nextrel, locoff, nlocrel = dy[14], dy[15], dy[16], dy[17]
        for i in range(nextrel):
            ra, w = struct.unpack_from("<iI", m.d, extoff + 8 * i)
            sn = w & 0xffffff
            ty, ln, pc = (w >> 28) & 0xf, (w >> 25) & 3, (w >> 24) & 1
            S = resolve(sn)
            if ty == 0 and ln == 3 and not pc:
                add = struct.unpack_from("<Q", img, ra)[0]
                struct.pack_into("<Q", img, ra, (S + add) & 0xffffffffffffffff)
                counts["ext64"] += 1
            elif ty == 2 and ln == 2 and pc:
                add = struct.unpack_from("<i", img, ra)[0]
                P = load + ra
                struct.pack_into("<i", img, ra, S + add - (P + 4) if S else 0)
                counts["branch"] += 1
            else:
                sys.exit("unhandled ext reloc %s in %s" % ((ty, ln, pc), e["bid"]))
        for i in range(nlocrel):
            ra, w = struct.unpack_from("<iI", m.d, locoff + 8 * i)
            ty, ln, pc = (w >> 28) & 0xf, (w >> 25) & 3, (w >> 24) & 1
            if not (ty == 0 and ln == 3 and not pc):
                sys.exit("unhandled local reloc in %s" % e["bid"])
            v = struct.unpack_from("<Q", img, ra)[0]
            struct.pack_into("<Q", img, ra, (v + load) & 0xffffffffffffffff)
            counts["rebase"] += 1

        # header: segment and section addresses, drop linkedit, mark in-cache
        hdr = img
        for sg in m.segs:
            lc = sg["lc"]
            if sg["name"] == "__LINKEDIT":
                struct.pack_into("<QQQQ", hdr, lc + 24, load + size, 0, 0, 0)
                continue
            struct.pack_into("<Q", hdr, lc + 24, sg["vm"] + load)
        for sc in m.sects:
            if sc["seg"] == "__LINKEDIT":
                continue
            struct.pack_into("<Q", hdr, sc["lc"] + 32, sc["addr"] + load)
        fl = struct.unpack_from("<I", hdr, 24)[0]
        struct.pack_into("<I", hdr, 24, fl | MH_DYLIB_IN_CACHE)
        kv = m.find_sym("_kmod_info")
        e["kmod"] = (kv + load) if kv is not None else 0
        if kv is not None:
            hdr_size = 32 + struct.unpack_from("<I", m.d, 20)[0]
            struct.pack_into("<QQQ", hdr, kv + 156, load, size, hdr_size)
            if e["bid"] in a.no_start:
                struct.pack_into("<QQ", hdr, kv + 180, 0, 0)

    for e in entries:
        if e["exe"] and not e.get("kmod"):
            print("NO _kmod_info symbol:", e["bid"], os.path.relpath(e["exe"], e["b"]["root"]))
    # assemble block and plist
    total = cur - base
    block = bytearray(total)
    for e in entries:
        if e["exe"]:
            off = e["load"] - base
            block[off:off + e["size"]] = e["img"]

    infos = []
    for e in entries:
        b = e["b"]
        d = dict(b["info"])
        for st in a.strip:
            sb, sk = st.split(":", 1)
            if sb == e["bid"]:
                d.pop(sk, None)
        for al in a.add_lib:
            ab, al_id, al_ver = al.split(":", 2)
            if ab == e["bid"]:
                for key in ("OSBundleLibraries", "OSBundleLibraries_x86_64"):
                    if key in d:
                        d[key] = dict(d[key])
                        d[key][al_id] = al_ver
                if "OSBundleLibraries" not in d and "OSBundleLibraries_x86_64" not in d:
                    d["OSBundleLibraries"] = {al_id: al_ver}
        for rn in a.rename:
            ro, rnw = rn.split("=", 1)
            if ro == e["bid"]:
                d["CFBundleIdentifier"] = rnw
        for st in a.set_true:
            sb, sk = st.split(":", 1)
            if sb == e["bid"]:
                d[sk] = True
        rel = os.path.relpath(b["path"], b["root"]).replace("\\", "/")
        d["_PrelinkBundlePath"] = "/System/Library/Extensions/" + rel
        if e["exe"]:
            d["_PrelinkExecutableRelativePath"] = os.path.relpath(e["exe"], b["path"]).replace("\\", "/")
            d["_PrelinkExecutableLoadAddr"] = e["load"]
            d["_PrelinkExecutableSize"] = e["size"]
            d["_PrelinkKmodInfo"] = e["kmod"]
        infos.append(d)
    plist = ('<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
             '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n' +
             xml({"_PrelinkInfoDictionary": infos}) + "\n</plist>\n\0").encode()
    plist_off = (total + PAGE - 1) & ~(PAGE - 1)
    blob = bytearray(plist_off) + plist
    blob[:total] = block
    ccs = [e for e in entries if e["bid"] == "com.apple.kec.corecrypto" and e["exe"]]
    cc_start = cc_kmod = 0
    if ccs:
        cc_start = ccs[0]["load"] + ccs[0]["m"].find_sym("_corecrypto_kext_start")
        cc_kmod = ccs[0]["kmod"]
    head = struct.pack("<QQQQQQQQ", 0x434b58414e41, base, total, plist_off, len(plist), cc_start, cc_kmod, len(blob))
    open(a.out, "wb").write(head + bytes(blob))
    with open(a.out + ".map", "w") as mf:
        for e in entries:
            if e["exe"]:
                mf.write("%s %#x %#x %s\n" % (e["bid"], e["load"], e["size"], e["exe"]))
    print("kexts selected: %d (with code: %d)" % (len(entries), sum(1 for e in entries if e["exe"])))
    print("block %#x bytes, plist %d bytes, relocs: %s" % (total, len(plist), counts))
    print("corecrypto start %#x kmod %#x" % (cc_start, cc_kmod))
    if missing:
        print("UNRESOLVED symbols (%d):" % len(missing))
        for nm, who in sorted(missing.items())[:40]:
            print("  ", nm, sorted(who)[:3])


main()
