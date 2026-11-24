#!/bin/bash
# boot an EFI partition image (e.g. from anathema-esp) as a usb stick in qemu, then print the
# kernel's debug buffer and the serial log. works on linux (OVMF) and macos (homebrew qemu edk2).
# usage: tools/xrun_esp.sh esp.img     env: WAIT (seconds, default 60), QCPU, QMACH, QSMP
set -e
IMG=$1
[ -f "$IMG" ] || { echo "usage: $0 esp.img" >&2; exit 2; }
for pair in "/usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_VARS_4M.fd" \
            "/opt/homebrew/share/qemu/edk2-x86_64-code.fd /opt/homebrew/share/qemu/edk2-i386-vars.fd" \
            "/usr/local/share/qemu/edk2-x86_64-code.fd /usr/local/share/qemu/edk2-i386-vars.fd"; do
  set -- $pair
  if [ -f "$1" ] && [ -f "$2" ]; then CODE=$1; VARS=$2; break; fi
done
[ -n "$CODE" ] || { echo "no OVMF/edk2 x86_64 firmware found" >&2; exit 1; }
RUN=$(mktemp -d "${TMPDIR:-/tmp}/xrun.XXXXXX")
cp "$VARS" "$RUN/vars.fd"
qemu-system-x86_64 -machine ${QMACH:-q35} -cpu ${QCPU:-Haswell} -m 2G -smp ${QSMP:-1} -accel tcg -display none -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file="$CODE" \
  -drive if=pflash,format=raw,file="$RUN/vars.fd" \
  -device qemu-xhci,id=xhci -drive file="$IMG",format=raw,if=none,id=usbstick,snapshot=on \
  -device usb-storage,bus=xhci.0,drive=usbstick,bootindex=0 \
  -serial file:"$RUN/serial.log" -monitor unix:"$RUN/qmon.sock",server,nowait >/dev/null 2>&1 &
QPID=$!
sleep "${WAIT:-60}"
python3 -I - "$RUN/qmon.sock" <<'EOF'
import socket, sys, time, re
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(3)
def cmd(c):
    s.sendall((c + "\n").encode()); time.sleep(1.0)
    try: return s.recv(1 << 20).decode(errors="replace")
    except Exception: return ""
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
    print("KERNEL DEBUG BUFFER:\n" + "".join("\n" if c == "\\n" else ("'" if c == "\\'" else c) for c in chars))
cmd("quit")
EOF
kill $QPID 2>/dev/null || true
echo "--- serial log ($RUN/serial.log):"
cat -v "$RUN/serial.log" | grep -a -v "^\^\[" | tail -${LINES_SHOWN:-60} | cut -c1-200
