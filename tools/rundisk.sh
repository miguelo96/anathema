#!/bin/bash
# boot a full gpt disk image from an emulated usb stick. env: QCPU, QSMP, WAIT, IMG
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/xvars.fd
rm -f /tmp/qmon.sock /tmp/xserial.log
qemu-system-x86_64 -machine pc -cpu ${QCPU:-Haswell} -m 2G -smp ${QSMP:-1} -accel tcg -display none -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=/tmp/xvars.fd \
  -device qemu-xhci,id=xhci -drive file=${IMG:-/tmp/anathema.img},format=raw,if=none,id=usbstick -device usb-storage,bus=xhci.0,drive=usbstick,bootindex=0 \
  ${QDBG:-} -serial file:/tmp/xserial.log -monitor unix:/tmp/qmon.sock,server,nowait >/dev/null 2>&1 &
QPID=$!
sleep "${WAIT:-600}"
kill $QPID 2>/dev/null
