#!/bin/sh
# Prepare the Ingot runtime root (CONTEXT.md: Runtime root). Runs in
# the systemd-based initramfs via ingot-root.service, after the state
# partition (root=PARTUUID=...) is mounted at /sysroot by the stock
# sysroot.mount, and before the stock switch-root.
#
# Kernel command line (embedded in the UKI):
#   root=PARTUUID=<state> rootfstype=btrfs   state partition
#   usr=PARTUUID=<slot>  usrfstype=erofs     active slot
. /lib/dracut-lib.sh 2>/dev/null || . /usr/lib/dracut-lib.sh || true
set -u
NEWROOT=/sysroot

fail() {
    echo "ingot-prepare: $*" >&2
    exit 1
}

resolve_dev() { # $1=PARTUUID=...|UUID=...|LABEL=... -> /dev/disk/...
    case $1 in
        PARTUUID=*) echo "/dev/disk/by-partuuid/${1#PARTUUID=}" ;;
        UUID=*) echo "/dev/disk/by-uuid/${1#UUID=}" ;;
        LABEL=*) echo "/dev/disk/by-label/${1#LABEL=}" ;;
        *) echo "$1" ;;
    esac
}

usr_arg=$(getarg usr=)
case $usr_arg in
    PARTUUID=*|UUID=*|LABEL=*) : ;;
    *) fail "no usr=PARTUUID= on the kernel command line" ;;
esac
usr_dev=$(resolve_dev "$usr_arg")
usr_type=$(getarg usrfstype=)
[ -n "$usr_type" ] || usr_type=erofs

root_arg=$(getarg root=)
case $root_arg in
    PARTUUID=*|UUID=*|LABEL=*) : ;;
    *) fail "no root=PARTUUID= (state partition) on the kernel command line" ;;
esac
var_dev=$(resolve_dev "$root_arg")

wait_dev() {
    wd_i=0
    while [ ! -e "$1" ]; do
        wd_i=$((wd_i + 1))
        [ "$wd_i" -gt 60 ] && return 1
        sleep 1
    done
    return 0
}
wait_dev "$usr_dev" || fail "slot device $usr_dev not found"
wait_dev "$var_dev" || fail "state device $var_dev not found"

# The filesystem modules are embedded in the initramfs (module-setup.sh)
# and loaded early by systemd-modules-load; load them explicitly as well.
modprobe -q erofs || fail "cannot load the erofs module"
modprobe -q btrfs || fail "cannot load the btrfs module"

# The state partition currently sits at $NEWROOT (sysroot.mount); the
# runtime root is a fresh tmpfs instead (CONTEXT.md: Runtime root).
sysroot_mounted=0
while read -r _sm_dev _sm_mpt _sm_rest; do
    [ "$_sm_mpt" = "$NEWROOT" ] && { sysroot_mounted=1; break; }
done < /proc/mounts
[ "$sysroot_mounted" = 1 ] || fail "state partition not mounted at $NEWROOT"
umount "$NEWROOT" || fail "cannot unmount $NEWROOT"
mount -t tmpfs -o mode=0755 tmpfs "$NEWROOT" || fail "tmpfs root mount failed"

# The active slot, read-only, at /usr.
mkdir -p "$NEWROOT/usr"
mount -t "$usr_type" -o ro "$usr_dev" "$NEWROOT/usr" \
    || fail "slot $usr_dev not mounted at $NEWROOT/usr"

# The state partition at /var.
mkdir -p "$NEWROOT/var"
mount "$var_dev" "$NEWROOT/var" || fail "state partition not mounted at $NEWROOT/var"

# Materialize the factory /etc defaults onto /var/lib/etc, per file and
# never clobbering: admin state created at runtime (or injected by the
# harness before a scenario boot) survives reboots and slot switches,
# while missing factory files are restored. /var/lib/etc may pre-exist
# (the harness drops fixtures there before boots), so this runs on every
# boot, not only the first.
mkdir -p "$NEWROOT/var/lib/etc"
cp -an "$NEWROOT/usr/share/factory/etc/." "$NEWROOT/var/lib/etc/" \
    || fail "cannot materialize /etc from the factory defaults"
mkdir -p "$NEWROOT/etc"
mount --bind "$NEWROOT/var/lib/etc" "$NEWROOT/etc" \
    || fail "cannot bind /etc over /var/lib/etc"

# usr symlinks and empty top-level directories.
ln -sfn usr/bin "$NEWROOT/bin"
ln -sfn usr/sbin "$NEWROOT/sbin"
ln -sfn usr/lib "$NEWROOT/lib"
ln -sfn usr/lib64 "$NEWROOT/lib64"
mkdir -p "$NEWROOT/opt" "$NEWROOT/srv" "$NEWROOT/media" "$NEWROOT/mnt" \
    "$NEWROOT/root" "$NEWROOT/home" "$NEWROOT/efi" \
    "$NEWROOT/dev" "$NEWROOT/proc" "$NEWROOT/sys" \
    "$NEWROOT/run" "$NEWROOT/tmp"

[ -x "$NEWROOT/usr/lib/systemd/systemd" ] \
    || fail "systemd binary missing from the slot"
echo "ingot-prepare: runtime root ready at $NEWROOT"
