"""stream-decode pbzx files into one output file. usage: pbzxcat.py out file..."""
import lzma, struct, sys
out = open(sys.argv[1], 'wb')
for path in sys.argv[2:]:
    with open(path, 'rb') as f:
        assert f.read(4) == b'pbzx', path
        f.read(8)
        while True:
            hdr = f.read(16)
            if not hdr:
                break
            usize, csize = struct.unpack('>QQ', hdr)
            data = f.read(csize)
            out.write(data if csize == usize else lzma.decompress(data, format=lzma.FORMAT_XZ))
    print('ok', path, flush=True)
out.close()
