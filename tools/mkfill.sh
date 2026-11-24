#!/bin/bash
# usage: mkfill.sh image file_to_install_as_/sbin/launchd
set -e
IMG=$1
LD=$2
cd ~/hfsvm
rm -rf rd && mkdir -p rd/bin rd/lib/modules rd/dev rd/proc rd/sys rd/mnt rd/payload
cp x/usr/bin/busybox rd/bin/busybox 2>/dev/null || cp $(find x -name busybox -type f | head -1) rd/bin/busybox
for m in $(find x -name "hfsplus.ko.zst" -o -name "nls_utf8.ko.zst"); do zstd -dq -f "$m" -o rd/lib/modules/$(basename ${m%.zst}); done
cp "$LD" rd/payload/launchd
cat > rd/init <<'EOF'
#!/bin/busybox sh
/bin/busybox --install -s /bin
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev
insmod /lib/modules/nls_utf8.ko
insmod /lib/modules/hfsplus.ko
mount -t hfsplus -o rw /dev/vda /mnt || { echo MOUNTFAIL; poweroff -f; }
mkdir -p /mnt/sbin /mnt/dev /mnt/private /mnt/usr /mnt/System /mnt/Library
cp /payload/launchd /mnt/sbin/launchd
ls -l /mnt/sbin
sync
umount /mnt
echo FILLDONE
poweroff -f
EOF
chmod +x rd/init
(cd rd && find . | cpio -o -H newc 2>/dev/null | gzip -1 > ../fill.cpio.gz)
timeout 120 qemu-system-x86_64 -m 512 -nographic -no-reboot -kernel x/boot/vmlinuz-7.0.0-44-generic -initrd fill.cpio.gz -append "console=ttyS0 quiet" -drive file=$IMG,format=raw,if=virtio 2>&1 | tail -15
