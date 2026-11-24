import sys, os
f=open(sys.argv[1],"rb"); pat=sys.argv[2:]
outdir="kdk_files"
while True:
    h=f.read(76)
    if len(h)<76 or h[:6]!=b"070707": break
    mode=int(h[18:24],8); ns=int(h[59:65],8); fs=int(h[65:76],8)
    name=f.read(ns)[:-1].decode()
    if name=="TRAILER!!!": break
    if pat and any((name==p[1:]) if p[0]=="=" else (p in name) for p in pat) and (mode&0o170000)==0o100000:
        p=os.path.join(outdir,name.lstrip("./")); os.makedirs(os.path.dirname(p),exist_ok=True)
        open(p,"wb").write(f.read(fs)); print("X",name,fs)
    else:
        f.seek(fs,1)
        if not pat and (mode&0o170000)==0o100000 and ("corecrypto" in name or "/Kernels/" in name): print(name,fs)
