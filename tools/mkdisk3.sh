#!/bin/bash
# usage: mkdisk3.sh out.img helper_hfs.img apfs.raw   gpt disk: esp + helper hfs+ root + apfs container
set -e
OUT=$1
HFS=$2
APFS=$3
HS=$(stat -c %s "$HFS")
AS=$(stat -c %s "$APFS")
ESP_MB=200
TOTAL=$(( (ESP_MB * 1024 * 1024) + HS + AS + 8 * 1024 * 1024 ))
truncate -s $TOTAL "$OUT"
sgdisk -o "$OUT" >/dev/null
sgdisk -n 1:2048:+${ESP_MB}M -t 1:EF00 -c 1:ESP "$OUT" >/dev/null
sgdisk -n 2:0:+$((HS / 1048576 + 1))M -t 2:AF00 -c 2:anathema-helper "$OUT" >/dev/null
sgdisk -n 3:0:0 -t 3:AF0A -c 3:anathema-apfs "$OUT" >/dev/null
sgdisk -p "$OUT" | tail -4
P1S=$(sgdisk -i 1 "$OUT" | awk '/First sector/{print $3}')
P2S=$(sgdisk -i 2 "$OUT" | awk '/First sector/{print $3}')
P3S=$(sgdisk -i 3 "$OUT" | awk '/First sector/{print $3}')
truncate -s $((ESP_MB * 1024 * 1024)) /tmp/esp_part.img
mkfs.fat -F32 /tmp/esp_part.img >/dev/null
mmd -i /tmp/esp_part.img ::/EFI ::/EFI/BOOT ::/anathema
mcopy -i /tmp/esp_part.img "$EFI" ::/EFI/BOOT/BOOTX64.EFI
for f in xkernel xkc.bin xcp.bin xcp2.bin xcp3.bin; do
  [ -f "$FILES/$f" ] && mcopy -i /tmp/esp_part.img "$FILES/$f" ::/anathema/$f
done
printf '%s' "$XARGS" > /tmp/xargs_disk.txt
mcopy -i /tmp/esp_part.img /tmp/xargs_disk.txt ::/anathema/xargs.txt
dd if=/tmp/esp_part.img of="$OUT" bs=512 seek=$P1S conv=notrunc status=none
dd if="$HFS" of="$OUT" bs=512 seek=$P2S conv=notrunc status=none
dd if="$APFS" of="$OUT" bs=512 seek=$P3S conv=notrunc status=none
echo "p1 $P1S p2 $P2S p3 $P3S"
ls -la "$OUT"
