#!/bin/bash
# parallel-safe usb boot test. env: TAG (name), IMG, QCPU, QSMP, WAIT, SNAP=1 (discard disk writes, lets several runs share one image)
TAG=${TAG:-t1}
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/xvars_$TAG.fd
rm -f /tmp/qmon_$TAG.sock /tmp/serial_$TAG.log
SNAPFLAG=""
[ -n "$SNAP" ] && SNAPFLAG="-snapshot"
qemu-system-x86_64 -machine pc -cpu ${QCPU:-Haswell} -m 2G -smp ${QSMP:-1} -accel tcg -display none -no-reboot $SNAPFLAG \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=/tmp/xvars_$TAG.fd \
  -device qemu-xhci,id=xhci -drive file=${IMG:-/tmp/two.img},format=raw,if=none,id=usbstick -device usb-storage,bus=xhci.0,drive=usbstick,bootindex=0 \
  -serial file:/tmp/serial_$TAG.log -monitor unix:/tmp/qmon_$TAG.sock,server,nowait >/dev/null 2>&1 &
QPID=$!
sleep "${WAIT:-600}"
kill $QPID 2>/dev/null
echo "done $TAG"
