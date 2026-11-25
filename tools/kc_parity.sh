#!/bin/bash
# check that the rust linker (anathema-kc) and kcbuild.py build the same xkc.bin from the same trees.
# usage: tools/kc_parity.sh KERNEL TAHOE_EXTENSIONS_DIR KDK27_EXTENSIONS_DIR OUTDIR
# runs both with the mkkc.sh options and the tools/kc-roots.txt roots. kcbuild.py walks folders in
# file system order and anathema-kc in sorted order, so the python run here uses a copy of
# kcbuild.py with a sorted walk (the only change); see docs/superpowers/specs/*kc-linker-setup*.
set -e
KERNEL=$1; EXT1=$2; EXT2=$3; OUT=$4
[ -n "$OUT" ] || { echo "usage: $0 KERNEL EXT1 EXT2 OUTDIR" >&2; exit 2; }
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT/python" "$OUT/rust"
python3 -I - "$HERE/kcbuild.py" "$OUT/kcbuild_sorted.py" <<'EOF'
import sys
src = open(sys.argv[1]).read()
walk = "        for dp, dn, fn in os.walk(root):\n"
assert src.count(walk) == 1, "kcbuild.py walk loop changed; update kc_parity.sh"
open(sys.argv[2], "w").write(src.replace(walk, walk + "            dn.sort()\n"))
EOF
ROOTS="com.apple.driver.AppleACPIPlatform com.apple.kec.corecrypto com.apple.kec.AppleEncryptedArchive
  com.apple.kec.XrtHostedXnu com.apple.kec.Libm com.apple.kec.Compression com.apple.kec.pthread
  com.apple.security.AppleImage4 com.apple.kext.CoreTrust $(grep -v '^#' "$HERE/kc-roots.txt")"
OPTS=(--kernel "$KERNEL" --ext "$EXT1" --ext "$EXT2"
  --prefer-last com.apple.kec.corecrypto --prefer-last com.apple.kec.Compression
  --prefer-last com.apple.kec.pthread --prefer-last com.apple.kec.Libm
  --prefer-last com.apple.kec.AppleEncryptedArchive --prefer-last com.apple.kec.XrtHostedXnu
  --prefer-last com.apple.security.AppleImage4
  --strip com.apple.security.AppleImage4:AppleSecurityExtension
  --strip com.apple.kext.CoreTrust:AppleSecurityExtension
  --codeless com.apple.driver.AppleMobileFileIntegrity
  --base 0xffffff8002000000)
python3 -I "$OUT/kcbuild_sorted.py" "${OPTS[@]}" --out "$OUT/python/xkc.bin" $ROOTS > "$OUT/python/stdout.txt"
cargo run -q --release -p anathema-kc --manifest-path "$HERE/../Cargo.toml" -- "${OPTS[@]}" --out "$OUT/rust/xkc.bin" $ROOTS > "$OUT/rust/stdout.txt"
status=0
for f in xkc.bin xkc.bin.map stdout.txt; do
  if cmp -s "$OUT/python/$f" "$OUT/rust/$f"; then echo "same: $f"; else echo "DIFFERENT: $f"; status=1; fi
done
shasum -a 256 "$OUT/rust/xkc.bin"
head -3 "$OUT/rust/stdout.txt"
exit $status
