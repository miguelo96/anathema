#!/bin/bash
# usage: mkdisk.sh out.img root_hfs.img   builds a gpt disk image: efi partition (loader + kernel files) + hfs+ root
set -e
OUT=$1
ROOT=$2
RSIZE=$(stat -c %s "$ROOT")
ESP_MB=200
TOTAL=$(( (ESP_MB * 1024 * 1024) + RSIZE + 4 * 1024 * 1024 ))
rm -f "$OUT"
truncate -s $TOTAL "$OUT"
sgdisk -o "$OUT" >/dev/null
sgdisk -n 1:2048:+${ESP_MB}M -t 1:EF00 -c 1:ESP "$OUT" >/dev/null
sgdisk -n 2:0:0 -t 2:AF00 -c 2:anathema-root "$OUT" >/dev/null
sgdisk -p "$OUT" | tail -4
P1S=$(sgdisk -i 1 "$OUT" | awk '/First sector/{print $3}')
P2S=$(sgdisk -i 2 "$OUT" | awk '/First sector/{print $3}')
P2E=$(sgdisk -i 2 "$OUT" | awk '/Last sector/{print $3}')
P1B=$((P1S * 512))
# esp
truncate -s $((ESP_MB * 1024 * 1024)) /tmp/esp_part.img
mkfs.fat -F32 /tmp/esp_part.img >/dev/null
mmd -i /tmp/esp_part.img ::/EFI ::/EFI/BOOT ::/anathema
mcopy -i /tmp/esp_part.img "$EFI" ::/EFI/BOOT/BOOTX64.EFI
for f in xkernel xkc.bin xcp.bin xcp2.bin; do
  [ -f "$FILES/$f" ] && mcopy -i /tmp/esp_part.img "$FILES/$f" ::/anathema/$f
done
printf '%s' "$XARGS" > /tmp/xargs_disk.txt
mcopy -i /tmp/esp_part.img /tmp/xargs_disk.txt ::/anathema/xargs.txt
dd if=/tmp/esp_part.img of="$OUT" bs=512 seek=$P1S conv=notrunc status=none
dd if="$ROOT" of="$OUT" bs=512 seek=$P2S conv=notrunc status=none
echo "p1 start $P1S, p2 start $P2S end $P2E"
ls -la "$OUT"
