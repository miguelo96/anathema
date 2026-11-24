"""decode a RIDIFF10 full-replacement patch whose body is a pbzx stream. usage: ridiff.py patch out"""
import lzma, struct, sys
f = open(sys.argv[1], 'rb')
d = f.read(0x80)
assert d[:8] == b'RIDIFF10'
pos = d.find(b'pbzx')
print('pbzx at', hex(pos))
f.seek(pos + 4)
flags = f.read(8)
out = open(sys.argv[2], 'wb')
total = 0
n = 0
while True:
    hdr = f.read(16)
    if len(hdr) < 16:
        break
    usize, csize = struct.unpack('>QQ', hdr)
    if csize == 0 or usize == 0 or csize > usize + 4096:
        print('stop at chunk', n, hex(usize), hex(csize), 'file pos', f.tell())
        break
    data = f.read(csize)
    if len(data) != csize:
        print('truncated at chunk', n)
        break
    if csize == usize:
        out.write(data)
        total += usize
    else:
        try:
            b = lzma.decompress(data, format=lzma.FORMAT_XZ)
        except Exception as e:
            print('lzma error at chunk', n, e, 'pos', f.tell())
            break
        out.write(b)
        total += len(b)
    n += 1
    if n % 200 == 0:
        print(n, total >> 20, 'MB', flush=True)
print('chunks', n, 'bytes', total, 'file pos', f.tell(), 'of', f.seek(0, 2))
