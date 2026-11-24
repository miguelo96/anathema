"""extract partitions from an uncompressed (raw/zero chunk) UDIF dmg. usage: udif.py dmg outprefix"""
import base64, plistlib, struct, sys
f = open(sys.argv[1], 'rb')
f.seek(0, 2)
size = f.tell()
f.seek(size - 512)
k = f.read(512)
assert k[:4] == b'koly', k[:4]
xoff, xlen = struct.unpack('>QQ', k[0xd8:0xe8])
f.seek(xoff)
pl = plistlib.loads(f.read(xlen))
blkx = pl['resource-fork']['blkx']
for ent in blkx:
    name = ent.get('Name') or ent.get('CFName')
    data = ent['Data']
    if isinstance(data, str):
        data = base64.b64decode(data)
    assert data[:4] == b'mish'
    first, count, doff = struct.unpack('>QQQ', data[8:32])
    nchunks, = struct.unpack('>I', data[0xc8:0xcc])
    print('partition', name, 'sectors', count, 'chunks', nchunks)
    out = open('%s.%s.raw' % (sys.argv[2], ent['ID']), 'wb')
    out.truncate(count * 512)
    for i in range(nchunks):
        c = data[0xcc + 40 * i:0xcc + 40 * (i + 1)]
        typ, _, sec, cnt, coff, clen = struct.unpack('>IIQQQQ', c)
        if typ == 0xffffffff:
            break
        if typ in (0, 2, 0x7ffffffe):
            continue
        if typ == 1:
            f.seek(doff + coff)
            out.seek(sec * 512)
            left = clen
            while left:
                b = f.read(min(left, 1 << 22))
                out.write(b)
                left -= len(b)
        else:
            print('unsupported chunk type', hex(typ))
            sys.exit(1)
    out.close()
