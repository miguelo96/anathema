#!/bin/sh
# runs as a launchd daemon in the helper hfs+ root: mounts the apfs recovery volume read-write and patches it
exec >/dev/console 2>&1
echo "patcher: start"
i=0
while [ ! -e /dev/disk1s1 ] && [ $i -lt 90 ]; do
	/bin/sleep 1
	i=$((i + 1))
done
echo "patcher: waited $i s"
/bin/ls /dev | /usr/bin/grep disk
/sbin/mount_apfs -o rw /dev/disk1s1 /mnt/apfs
echo "patcher: mount_apfs rc=$?"
/sbin/mount
if [ -d /mnt/apfs/usr/bin ]; then
	echo "patcher: root volume visible"
	/bin/rm -f /mnt/apfs/usr/bin/pmset
	echo '#!/bin/sh' > /mnt/apfs/usr/bin/pmset
	echo 'exit 0' >> /mnt/apfs/usr/bin/pmset
	/bin/chmod 755 /mnt/apfs/usr/bin/pmset
	echo "patcher: pmset stub written"
	/bin/ls -la /mnt/apfs/usr/bin/pmset
	for f in com.apple.WindowServer.plist com.apple.audiomxd.plist; do
		P=/mnt/apfs/System/Library/LaunchDaemons/$f
		if [ -f $P ]; then
			/usr/bin/sed -e '/_Conclave/{N;d;}' $P > /mnt/apfs/System/Library/LaunchDaemons/$f.new
			/bin/mv -f /mnt/apfs/System/Library/LaunchDaemons/$f.new $P
			echo "patcher: stripped _Conclave from $f"
		fi
	done
else
	echo "patcher: no root volume"
fi
/bin/sync
echo "patcher: done, rebooting"
/sbin/reboot
