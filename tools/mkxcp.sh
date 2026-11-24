#!/bin/bash
# build xcp.bin, the two commpage map stubs anathema-xboot installs, from tools/commpage_stub.c:
# .text.a (anx_commpage_init) at offset 0 and .text.b (anx_commpage_text_init) at 0x100.
# usage: tools/mkxcp.sh [OUT]   (default crates/anathema-setup/assets/xcp.bin)
# works with apple clang or linux clang/gcc (CC=...); needs python3 for the elf section copy.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${1:-$HERE/../crates/anathema-setup/assets/xcp.bin}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
${CC:-clang} -target x86_64-unknown-linux-gnu -O2 -ffreestanding -fno-pic -fno-stack-protector \
  -mno-red-zone -mcmodel=kernel -c "$HERE/commpage_stub.c" -o "$TMP/xcp.o"
python3 -I - "$TMP/xcp.o" "$OUT" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
assert d[:4] == b"\x7fELF" and d[4] == 2, "not an elf64 object"
shoff, = struct.unpack_from("<Q", d, 0x28)
shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 0x3a)
secs = [struct.unpack_from("<IIQQQQ", d, shoff + i * shentsize) for i in range(shnum)]
stroff = secs[shstrndx][4]
def name(s):
    end = d.index(b"\0", stroff + s[0])
    return d[stroff + s[0]:end].decode()
by = {name(s): s for s in secs}
for n in by:
    if n.startswith(".rel") and n.split(".text", 1)[-1] in (".a", ".b", ""):
        sys.exit("%s has relocations; the stubs must be position independent" % n)
if by.get(".text") and by[".text"][5]:
    sys.exit(".text is not empty: a helper was not inlined and would need a relocation")
a = d[by[".text.a"][4]:by[".text.a"][4] + by[".text.a"][5]]
b = d[by[".text.b"][4]:by[".text.b"][4] + by[".text.b"][5]]
assert len(a) <= 0x100, ".text.a is %#x bytes, more than 0x100" % len(a)
open(sys.argv[2], "wb").write(a + bytes(0x100 - len(a)) + b)
print("%s: %#x bytes (.text.a %#x, .text.b %#x)" % (sys.argv[2], 0x100 + len(b), len(a), len(b)))
PY
