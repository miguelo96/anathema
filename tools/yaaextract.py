"""extract an apple archive (YAA1) stream to a directory. usage: yaaextract.py archive outdir [prefix ...]
only entries whose path starts with one of the prefixes are written (all when none are given)."""
import os, struct, sys

FIX = {'F': 4, 'G': 20, 'H': 32, 'I': 48, 'J': 64, 'T': 12, 'S': 8}

def entries(f):
    while True:
        h = f.read(6)
        if len(h) < 6:
            return
        assert h[:4] == b'YAA1', h
        hs, = struct.unpack('<H', h[4:])
        body = f.read(hs - 6)
        i = 0
        fields = {}
        blobs = []
        while i < len(body):
            k = body[i:i + 3].decode()
            t = chr(body[i + 3])
            i += 4
            if t in '1248':
                n = int(t)
                fields[k] = int.from_bytes(body[i:i + n], 'little')
                i += n
            elif t == 'P':
                n, = struct.unpack_from('<H', body, i)
                fields[k] = body[i + 2:i + 2 + n]
                i += 2 + n
            elif t in 'ABC':
                n = {'A': 2, 'B': 4, 'C': 8}[t]
                sz = int.from_bytes(body[i:i + n], 'little')
                i += n
                blobs.append((k, sz))
            elif t in FIX:
                fields[k] = body[i:i + FIX[t]]
                i += FIX[t]
            elif t == '*':
                fields[k] = True
            else:
                raise ValueError('field %s%s' % (k, t))
        yield fields, blobs

def main():
    arc, out = sys.argv[1], sys.argv[2]
    prefixes = [p.encode() for p in sys.argv[3:]]
    n = 0
    skipped = 0
    f = open(arc, 'rb')
    for fields, blobs in entries(f):
        blob_data = {}
        for k, sz in blobs:
            blob_data[k] = (f.tell(), sz)
            f.seek(sz, 1)
        pat = fields.get('PAT')
        if pat is None and 'PAT' in blob_data:
            cur = f.tell()
            p0, sz0 = blob_data['PAT']
            f.seek(p0)
            pat = f.read(sz0)
            f.seek(cur)
        if pat is None:
            pat = b''
        typ = chr(fields.get('TYP', 0)) if isinstance(fields.get('TYP'), int) else '?'
        path = pat.decode('utf-8', 'surrogateescape')
        if prefixes and not any(pat.startswith(p) for p in prefixes):
            skipped += 1
            continue
        dest = os.path.join(out, path)
        mode = fields.get('MOD', 0o755 if typ == 'D' else 0o644) & 0o7777
        if typ == 'D':
            os.makedirs(dest, exist_ok=True)
            os.chmod(dest, mode | 0o700)
        elif typ == 'L':
            tgt = fields.get('LNK')
            if 'LNK' in blob_data:
                p, sz = blob_data['LNK']
                cur = f.tell()
                f.seek(p)
                tgt = f.read(sz)
                f.seek(cur)
            if tgt is not None:
                os.makedirs(os.path.dirname(dest), exist_ok=True)
                if os.path.lexists(dest):
                    os.remove(dest)
                os.symlink(tgt.decode('utf-8', 'surrogateescape'), dest)
        elif typ == 'F':
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, 'wb') as w:
                if 'DAT' in blob_data:
                    p, sz = blob_data['DAT']
                    cur = f.tell()
                    f.seek(p)
                    left = sz
                    while left:
                        b = f.read(min(left, 1 << 22))
                        w.write(b)
                        left -= len(b)
                    f.seek(cur)
            os.chmod(dest, mode)
        n += 1
        if n % 20000 == 0:
            print('extracted', n, flush=True)
    print('done', n, 'skipped', skipped)

main()
