#!/bin/sh
# Ingot unattended installer (live ISO, spec 10.2, issue #8). Runs on
# the live system after multi-user (ingot-live-install.service, armed
# by the live initramfs; the installed system never carries the unit).
#
# With a declarative config disk present, the installer runs
# unattended against the target disk named in the config (the config
# is the authorization, spec 11.6.2): no human input. Without one,
# the live session simply continues (rescue use, spec 10.1).
#
# Evidence: the installer's plan and result lines reach the console
# via the journal (the unit runs with journal output, forwarded to
# the console), the machine-readable install log lands in
# /run/ingot-work/install.log (and on the installed system at
# /var/lib/ingot/install.log on success), and this script writes the
# INGOT-LIVE-INSTALL marker lines directly to the serial console.
set -u
CONFIG_MOUNT=/run/ingot-config
WORK=/run/ingot-work

mark() {
    # Marker lines on the serial console (headless terminal,
    # spec 10.2): the harness and the operator read these.
    printf 'INGOT-LIVE-INSTALL: %s\n' "$*" | tee /dev/ttyS0 >/dev/null || true
}

# --- 1. find the config disk ----------------------------------------
# The config disk is a block device whose filesystem root carries
# ingot-install.toml. Scan the block devices (the QEMU virtio disks
# are /dev/vd*; the ISO cdrom is /dev/sr0 and never carries the
# config - the ISO itself has no ingot-install.toml).
config_dev=""
mkdir -p "$CONFIG_MOUNT"
for d in /sys/block/*; do
    n=$(basename "$d")
    case $n in loop* | ram* | dm-* | md*) continue ;; esac
    dev="/dev/$n"
    [ -b "$dev" ] || continue
    if mount -o ro,noexec,nosuid "$dev" "$CONFIG_MOUNT" 2>/dev/null; then
        if [ -f "$CONFIG_MOUNT/ingot-install.toml" ]; then
            config_dev=$dev
            break
        fi
        umount "$CONFIG_MOUNT" 2>/dev/null || true
    fi
done

if [ -z "$config_dev" ]; then
    mark "no config disk found; live session continues (no install)"
    printf 'no-config-disk\n' > /run/ingot-live-install.status
    exit 0
fi

cfg="$CONFIG_MOUNT/ingot-install.toml"
echo "ingot-live-install: config disk at $config_dev (config $cfg)" >&2
mark "config disk at $config_dev; starting unattended install"

# --- 2. run the installer --------------------------------------------
# The config is the authorization (declarative mode, spec 11.3/11.6.2).
# The payload carries the installer and its tooling (systemd-repart,
# the mkfs binaries, dd); the live source is the running release (the
# config's [source] mode = "live" names the ISO media). The working
# directory holds the install log and the repart definitions.
mkdir -p "$WORK"
/usr/bin/ingot-installer "$cfg" --work "$WORK" > /run/ingot-installer.out 2>&1
rc=$?
cat /run/ingot-installer.out > /dev/ttyS0 2>/dev/null || true

if [ $rc -eq 0 ]; then
    mark "complete (installer exit 0)"
    printf 'complete\n' > /run/ingot-live-install.status
else
    mark "failed (installer exit $rc; see $WORK/install.log and the console journal)"
    cat "$WORK/install.log" > /dev/ttyS0 2>/dev/null || true
    printf 'failed rc=%s\n' "$rc" > /run/ingot-live-install.status
fi

# The install is done (or failed). The live system stays up: on the
# harness the monitor drives the reboot; on a real machine the
# operator reboots into the installed system.
exit $rc
