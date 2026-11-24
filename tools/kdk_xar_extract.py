import struct, zlib, sys, xml.etree.ElementTree as ET
f=open(sys.argv[1],"rb")
magic,hsz,ver,toc_c,toc_u,ck=struct.unpack(">4sHHQQI",f.read(28))
f.seek(hsz); toc=zlib.decompress(f.read(toc_c)); heap=hsz+toc_c
root=ET.fromstring(toc)
def walk(el,path):
    for fe in el.findall("file"):
        name=fe.find("name").text; p=path+"/"+name if path else name
        d=fe.find("data")
        if d is not None and fe.find("type").text=="file":
            yield p,int(d.find("offset").text),int(d.find("length").text),d.find("encoding").attrib.get("style","")
        yield from walk(fe,p)
for p,off,ln,enc in walk(root.find("toc"),""):
    print(p,off,ln,enc)
    if p==sys.argv[2]:
        f.seek(heap+off); open(sys.argv[3],"wb").write(f.read(ln)); print("wrote",sys.argv[3])
