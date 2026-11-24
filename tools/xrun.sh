#!/bin/bash
# build the loader, make a test disk, boot the intel kernel in qemu, then read the kernel's panic text
set -e
cd ~/src/anathema-core
CARGO_TARGET_DIR=~/xboot-target cargo build --release -p anathema-xboot --target x86_64-unknown-uefi 2>&1 | grep -E "^error" -A10 | head -30 || true
EFI=~/xboot-target/x86_64-unknown-uefi/release/anathema-xboot.efi
cd /tmp
rm -f xesp.img
dd if=/dev/zero of=xesp.img bs=1M count=96 status=none
mkfs.fat -F32 xesp.img >/dev/null
mmd -i xesp.img ::/EFI ::/EFI/BOOT ::/anathema
mcopy -i xesp.img "$EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i xesp.img ~/anathema-work/x86k/kernel ::/anathema/xkernel
[ -f ~/anathema-work/xcp.bin ] && mcopy -i xesp.img ~/anathema-work/xcp.bin ::/anathema/xcp.bin
[ -f ~/anathema-work/xcp2.bin ] && mcopy -i xesp.img ~/anathema-work/xcp2.bin ::/anathema/xcp2.bin
[ -f ~/anathema-work/xkc.bin ] && [ -z "$NOKEXT" ] && mcopy -i xesp.img ~/anathema-work/xkc.bin ::/anathema/xkc.bin
if [ -n "$XARGS" ]; then printf '%s' "$XARGS" > /tmp/xargs.txt; mcopy -i xesp.img /tmp/xargs.txt ::/anathema/xargs.txt; fi
if [ -n "$XTSC" ]; then : > /tmp/xpatch-tsc; mcopy -i xesp.img /tmp/xpatch-tsc ::/anathema/xpatch-tsc; fi
if [ -n "$XAMD" ]; then : > /tmp/xpatch-amd; mcopy -i xesp.img /tmp/xpatch-amd ::/anathema/xpatch-amd; fi
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/xvars.fd
rm -f /tmp/qmon.sock /tmp/xserial.log
qemu-system-x86_64 -machine ${QMACH:-q35} -cpu ${QCPU:-Haswell} -m 2G -smp ${QSMP:-1} -accel tcg -display none -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=/tmp/xvars.fd \
  -drive file=/tmp/xesp.img,format=raw,if=virtio \
  ${QDBG:-} -serial file:/tmp/xserial.log -monitor unix:/tmp/qmon.sock,server,nowait >/dev/null 2>&1 &
QPID=$!
sleep "${WAIT:-40}"
python3 - <<'EOF'
import socket, time, re
s = socket.socket(socket.AF_UNIX); s.connect("/tmp/qmon.sock"); s.settimeout(3)
def cmd(c):
    s.sendall((c + "\n").encode()); time.sleep(1.0)
    try: return s.recv(1 << 20).decode(errors="replace")
    except Exception as e: return ""
cmd("")
print("REGS:", re.sub(r"\s+", " ", cmd("info registers"))[:400])
base = cmd("x/gx 0xffffff8000e99b08"); ptr = cmd("x/gx 0xffffff8000e99b10")
def val(t):
    m = re.findall(r"ffffff[0-9a-f]+: (0x[0-9a-f]+)", t); return int(m[-1], 16) if m else 0
b, p = val(base), val(ptr)
print("debug_buf", hex(b), hex(p))
n = max(0, min(p - b, 1500)) if b else 0
if n:
    out = cmd("x/%dbc %s" % (n, hex(b)))
    chars = re.findall(r"'(\\x[0-9a-f]{2}|\\n|\\t|\\\\|\\'|.)'", out)
    txt = "".join("\n" if c == "\\n" else ("'" if c == "\\'" else c) for c in chars)
    print("KERNEL DEBUG BUFFER:\n" + txt)
cmd("quit")
EOF
kill $QPID 2>/dev/null || true
echo "--- serial log:"
cat -v /tmp/xserial.log | grep -a -v "^\^\[" | tail -40 | cut -c1-200
