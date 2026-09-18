#!/bin/sh
# Prepare the Ingot live runtime root (spec 10: live ISO). Runs in the
# systemd-based initramfs via ingot-live-root.service, when the live
# UKI's command line carries root=live (the stock dracut generators
# ignore that value: no root device is expected, so the stock flow
# reaches initrd-root-fs.target with /sysroot still an empty
# directory, and the stock switch-root moves into whatever this
# script builds there).
#
# Kernel command line (embedded in the live UKI):
#   root=live                          live-mode marker (no device)
#
# Live root layout (spec 10.4-10.5: the live environment is ephemeral
# by default - everything writable is tmpfs-backed and resets on
# reboot):
#   /                    tmpfs
#   /usr                 the ISO's LiveOS/rootfs.erofs, read-only
#                        (the release slot payload: same userland,
#                        admin tools, and installer as the installed
#                        system)
#   /var                 tmpfs-backed directory (journal + /etc
#                        backing store live here, in RAM)
#   /etc                 bind over /var/lib/etc (factory defaults,
#                        materialized here per live boot)
#   /media/ingot-iso     the ISO media, read-only (the release
#                        artifacts the installer consumes live here)
#
# The console root login is opened for the live session (the factory
# baseline ships root with a locked password); the network root
# login stays password-closed (sshd's PermitRootLogin default is
# prohibit-password; the key-based rescue key is the live SSH path).
. /lib/dracut-lib.sh 2>/dev/null || . /usr/lib/dracut-lib.sh || true
set -u
NEWROOT=/sysroot
ISO_LABEL=INGOTLIVE
PROBE=/run/ingot-iso-probe

fail() { echo "ingot-live-prepare: $*" >&2; exit 1; }

modprobe -q iso9660 || fail "cannot load the iso9660 module"
modprobe -q erofs || fail "cannot load the erofs module"

# --- discover the ISO media ------------------------------------------
# The release ISO is the ISO9660 volume labelled ISO_LABEL carrying
# LiveOS/rootfs.erofs. udev names it under by-label when it is up;
# fall back to scanning the block devices (the QEMU cdrom is /dev/sr0).
iso_dev=""
if [ -e "/dev/disk/by-label/$ISO_LABEL" ]; then
    iso_dev="/dev/disk/by-label/$ISO_LABEL"
fi
if [ -z "$iso_dev" ]; then
    mkdir -p "$PROBE"
    for d in /sys/block/*; do
        n=$(basename "$d")
        case $n in loop*|ram*|dm-*|md*) continue ;; esac
        dev="/dev/$n"
        [ -b "$dev" ] || continue
        if mount -t iso9660 -o ro "$dev" "$PROBE" 2>/dev/null; then
            if [ -f "$PROBE/LiveOS/rootfs.erofs" ]; then
                iso_dev=$dev
            fi
            umount "$PROBE" 2>/dev/null || true
        fi
    done
    rmdir "$PROBE" 2>/dev/null || true
fi
[ -n "$iso_dev" ] || fail "no Ingot ISO media found (iso9660, label $ISO_LABEL)"
echo "ingot-live-prepare: ISO media at $iso_dev"

# --- the runtime root -------------------------------------------------
# The ISO media under the runtime root (read-only), and the live
# payload from it: a file-backed erofs mount at /usr (the payload is
# a file on the ISO media, not a block device).
mountpoint -q "$NEWROOT" || mount -t tmpfs -o mode=0755 tmpfs "$NEWROOT" \
    || fail "tmpfs root mount failed"
mkdir -p "$NEWROOT/media/ingot-iso"
mount -t iso9660 -o ro "$iso_dev" "$NEWROOT/media/ingot-iso" \
    || fail "ISO media not mounted at $NEWROOT/media/ingot-iso"
[ -f "$NEWROOT/media/ingot-iso/LiveOS/rootfs.erofs" ] \
    || fail "live payload missing from the ISO media"
mkdir -p "$NEWROOT/usr"
mount -t erofs -o ro \
    "$NEWROOT/media/ingot-iso/LiveOS/rootfs.erofs" "$NEWROOT/usr" \
    || fail "live payload not mounted at $NEWROOT/usr"

# The writable state: tmpfs-backed /var; the factory /etc defaults
# materialized onto /var/lib/etc per live boot (ephemeral: a fresh
# copy on every boot, spec 10.5).
mkdir -p "$NEWROOT/var/lib/etc"
cp -a "$NEWROOT/usr/share/factory/etc/." "$NEWROOT/var/lib/etc/" \
    || fail "cannot materialize /etc from the factory defaults"
# /etc over /var/lib/etc (same model as the installed runtime root:
# the writable state holds the /etc content; without the bind the
# live systemd would see an empty /etc - no users, no enabled
# units, no machine-id).
mkdir -p "$NEWROOT/etc"
mount --bind "$NEWROOT/var/lib/etc" "$NEWROOT/etc" \
    || fail "cannot bind /etc over /var/lib/etc"
# The factory fstab names the installed machine's state partitions
# (the /home and /efi PARTUUIDs); the live system has no such
# devices (its state is the tmpfs root). Drop it: nothing on the
# live session mounts from fstab.
rm -f "$NEWROOT/var/lib/etc/fstab"


# Live console root: unlock the factory baseline's locked root
# password for the ephemeral live session only (the factory shadow
# entry ships locked). Network root login stays password-closed:
# sshd's PermitRootLogin default is prohibit-password; the key-based
# rescue path is the live SSH authorized_keys below.
shadow="$NEWROOT/var/lib/etc/shadow"
[ -f "$shadow" ] || fail "no shadow file in the factory baseline"
grep -q '^root:' "$shadow" || fail "no root shadow entry"
{
    while IFS=: read -r name pass rest; do
        if [ "$name" = root ]; then
            printf 'root::%s\n' "$rest"
        else
            printf '%s:%s:%s\n' "$name" "$pass" "$rest"
        fi
    done < "$shadow"
} > "$shadow.new" || fail "cannot unlock the console root"
mv "$shadow.new" "$shadow"

# Live identity: the hostname the console prompt carries.
printf 'ingot-live\n' > "$NEWROOT/var/lib/etc/hostname"

# Live SSH rescue (spec 10.2: the ISO offers SSH for remote workflows,
# headless). The slot payload carries the repo's live test key
# (tools/keys/live.key; the public half ships in the image at
# usr/share/ingot/live-ssh-key.pub). sshd's PermitRootLogin stays at
# the default prohibit-password (no password auth); the key opens
# key-based root login into the ephemeral live session only - the
# materialized /root is tmpfs, so the installed system never carries
# this authorized_keys.
live_key=/usr/share/ingot/live-ssh-key.pub
[ -r "$NEWROOT$live_key" ] || fail "live SSH key missing from the payload"
mkdir -p "$NEWROOT/root/.ssh" && chmod 700 "$NEWROOT/root/.ssh" \
    || fail "cannot create the live /root/.ssh"
cp "$NEWROOT$live_key" "$NEWROOT/root/.ssh/authorized_keys" \
    && chmod 600 "$NEWROOT/root/.ssh/authorized_keys" \
    || fail "cannot install the live SSH authorized_keys"

# Arm the unattended installer trigger: when the system reaches
# multi-user, live-install.sh looks for a config disk and drives the
# installer against it (spec 10.2, issue #8: the declarative config
# disk auto-triggers the installer).
mkdir -p "$NEWROOT/var/lib/etc/systemd/system/multi-user.target.wants"
cat > "$NEWROOT/var/lib/etc/systemd/system/ingot-live-install.service" <<'UNIT'
# Ingot unattended installer (live ISO, spec 10.2): after the live
# system reaches multi-user, look for a declarative config disk and
# drive the installer against it. Dropped into the live /etc by the
# initramfs (the installed system never carries this unit).
[Unit]
Description=Ingot unattended installer (config disk auto-trigger)
After=multi-user.target
Wants=multi-user.target

[Service]
Type=oneshot
ExecStart=/usr/lib/ingot/live-install.sh
StandardOutput=journal
StandardError=journal
UNIT
ln -s ../ingot-live-install.service \
    "$NEWROOT/var/lib/etc/systemd/system/multi-user.target.wants/ingot-live-install.service" \
    || fail "cannot enable the live installer trigger"

# A fresh machine identity per live boot (ephemeral, spec 10.5). An
# initialized machine-id keeps systemd out of the first-boot flow.
hex=""
while [ ${#hex} -lt 32 ]; do
    hex="$hex$((RANDOM % 16))$((RANDOM % 16))"
done
printf '%s\n' "$hex" > "$NEWROOT/var/lib/etc/machine-id"

# usr symlinks and empty top-level directories (CONTEXT.md: runtime
# root; the live /var is a tmpfs-backed directory, not a partition).
ln -sfn usr/bin "$NEWROOT/bin"
ln -sfn usr/sbin "$NEWROOT/sbin"
ln -sfn usr/lib "$NEWROOT/lib"
ln -sfn usr/lib64 "$NEWROOT/lib64"
mkdir -p "$NEWROOT/opt" "$NEWROOT/srv" "$NEWROOT/media" "$NEWROOT/mnt" \
    "$NEWROOT/root" "$NEWROOT/home" "$NEWROOT/efi" \
    "$NEWROOT/dev" "$NEWROOT/proc" "$NEWROOT/sys" \
    "$NEWROOT/run" "$NEWROOT/tmp"

[ -x "$NEWROOT/usr/lib/systemd/systemd" ] \
    || fail "systemd binary missing from the live payload"
echo "ingot-live-prepare: live root ready at $NEWROOT (media $iso_dev)"
