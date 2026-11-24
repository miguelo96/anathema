import os, re, subprocess, sys
dmg = os.path.expanduser("~/tahoe/su/AssetData/usr/standalone/update/ramdisk/x86_64SURamDisk.dmg")
root = os.path.expanduser("~/tahoe/su/ramroot")
out = subprocess.run(["7z", "l", "-slt", dmg], capture_output=True, text=True).stdout
fixed = 0
for block in out.split("\n\n"):
    d = {}
    for line in block.splitlines():
        if " = " in line:
            k, v = line.split(" = ", 1)
            d[k] = v
    if d.get("Method") == "symlink" and d.get("Symbolic Link") and d.get("Path"):
        p = os.path.join(root, d["Path"])
        t = d["Symbolic Link"]
        if os.path.islink(p) and os.readlink(p) == t:
            continue
        if os.path.lexists(p):
            os.remove(p)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        os.symlink(t, p)
        fixed += 1
print("links fixed", fixed)
