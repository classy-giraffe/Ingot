#!/bin/sh
# Ingot dracut module: prepares the Ingot runtime root inside the
# (systemd-based) initramfs, in the stock switch-root flow.
#
# Boot flow (CONTEXT.md: Runtime root):
#   root=PARTUUID=<state> rootfstype=btrfs -> stock dracut systemd
#     initramfs: the rootfs-generator creates sysroot.mount, which
#     mounts the state partition on /sysroot.
#   usr=PARTUUID=<slot> usrfstype=erofs -> consumed by
#     ingot-root.service (below), which runs after sysroot.mount and
#     before the stock initrd-cleanup.service that isolates the
#     switch-root target: the runtime root is a tmpfs on which the
#     slot's erofs is mounted read-only at /usr, the state partition
#     at /var, /etc is bound over /var/lib/etc, plus the usr symlinks
#     and empty top-level directories. The stock initrd-switch-root
#     then moves into it.
#
# The filesystem modules (erofs, btrfs) are embedded here and loaded
# early by the stock systemd-modules-load via /etc/modules-load.d.

check() {
    # Always included: the module carries the runtime-root preparation.
    return 0
}

depends() {
    echo systemd
}

install() {
    # filesystem modules for the state partition and the active slot
    hostonly='' instmods erofs btrfs
    inst_simple "$moddir/99-ingot.conf" /etc/modules-load.d/99-ingot.conf
    inst_script "$moddir/prepare-root.sh" /usr/lib/ingot/prepare-root.sh
    inst "$moddir/ingot-root.service" /etc/systemd/system/ingot-root.service
    mkdir -p "$initdir/etc/systemd/system/initrd.target.wants"
    ln -s ../ingot-root.service \
        "$initdir/etc/systemd/system/initrd.target.wants/ingot-root.service"
}
