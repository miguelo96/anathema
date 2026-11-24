#!/bin/bash
# usage: mkroot.sh image tarfile   (fills an hfs+ image with a tar of a root tree through a helper linux vm)
set -e
IMG=$1
TAR=$2
cd ~/hfsvm
rm -rf rd2 && mkdir -p rd2/bin rd2/lib/modules rd2/dev rd2/proc rd2/sys rd2/mnt
cp x/usr/bin/busybox rd2/bin/busybox 2>/dev/null || cp $(find x -name busybox -type f | head -1) rd2/bin/busybox
for m in $(find x -name "hfsplus.ko.zst" -o -name "nls_utf8.ko.zst"); do zstd -dq -f "$m" -o rd2/lib/modules/$(basename ${m%.zst}); done
cat > rd2/init <<'EOF'
#!/bin/busybox sh
/bin/busybox --install -s /bin
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev
insmod /lib/modules/nls_utf8.ko
insmod /lib/modules/hfsplus.ko
mount -t hfsplus -o rw /dev/vda /mnt || { echo MOUNTFAIL; poweroff -f; }
tar xf /dev/vdb -C /mnt 2>&1 | tail -5
sync
df /mnt
umount /mnt
echo FILLDONE
poweroff -f
EOF
chmod +x rd2/init
(cd rd2 && find . | cpio -o -H newc 2>/dev/null | gzip -1 > ../fill2.cpio.gz)
timeout 1500 qemu-system-x86_64 -m 1024 -nographic -no-reboot -kernel x/boot/vmlinuz-7.0.0-44-generic -initrd fill2.cpio.gz -append "console=ttyS0 quiet" -drive file=$IMG,format=raw,if=virtio -drive file=$TAR,format=raw,if=virtio,readonly=on 2>&1 | tail -15
