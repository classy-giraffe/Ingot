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
# Live ISO boot flow (spec 10.3: the live UKI carries root=live):
#   the stock generators expect no root device, so initrd-root-fs
#   target is reached with /sysroot still an empty directory;
#   ingot-live-root.service (condition: root=live) then builds the
#   live runtime root itself - tmpfs /, the ISO's live erofs payload
#   at /usr, the ISO media mounted read-only, and the factory /etc
#   materialized per boot (ephemeral, spec 10.5).
#
# The filesystem modules (erofs, btrfs, iso9660) are embedded here
# and loaded early by the stock systemd-modules-load via
# /etc/modules-load.d.

check() {
    # Always included: the module carries the runtime-root preparation.
    return 0
}

depends() {
    echo systemd
}

install() {
    # filesystem modules for the state partition, the active slot
    # and the live ISO media (spec 10.3)
    hostonly='' instmods erofs btrfs iso9660
    inst_simple "$moddir/99-ingot.conf" /etc/modules-load.d/99-ingot.conf
    inst_script "$moddir/prepare-root.sh" /usr/lib/ingot/prepare-root.sh
    inst "$moddir/ingot-root.service" /etc/systemd/system/ingot-root.service
    inst_script "$moddir/prepare-live-root.sh" /usr/lib/ingot/prepare-live-root.sh
    inst "$moddir/ingot-live-root.service" /etc/systemd/system/ingot-live-root.service
    # the stock 77dracut-systemd cmdline hook (dracut-cmdline.service)
    # dies on a root value it does not recognize before the live root
    # service can run; mark root=live handled (sourced before the
    # stock check via the script's source_hook cmdline)
    inst_hook cmdline 30 "$moddir/parse-live-root.sh"
    mkdir -p "$initdir/etc/systemd/system/initrd.target.wants"
    ln -s ../ingot-root.service \
        "$initdir/etc/systemd/system/initrd.target.wants/ingot-root.service"
    ln -s ../ingot-live-root.service \
        "$initdir/etc/systemd/system/initrd.target.wants/ingot-live-root.service"
}
