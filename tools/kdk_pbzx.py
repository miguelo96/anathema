import struct, lzma, sys
f=open(sys.argv[1],"rb"); out=open(sys.argv[2],"wb")
assert f.read(4)==b"pbzx"
flags=struct.unpack(">Q",f.read(8))[0]
while flags & (1<<24):
    flags,ln=struct.unpack(">QQ",f.read(16))
    d=f.read(ln)
    out.write(lzma.decompress(d) if d[:6]==b"\xfd7zXZ\x00" else d)
