# T1 resume note (2026-09-17)

State of issue #15 (first bootable slot). Read before starting T2+.

## Where things stand

**T1 complete.** All acceptance criteria verified; issue #15 closed.

- **Compose archive:** full mirror of the pinned compose
  Fedora-Rawhide-20260916.n.0 at `/home/tommy/ingot/composes`
  (67022 files, 108G, manifest at `composes/compose/manifest.json`).
  Rebuilds use `tools/build.sh --local` (fully offline dnf).
- **Build chain:** `tools/build.sh` (or `sudo /usr/local/bin/ingot-build`)
  runs pin checks, mkosi (single main image, disk output), split-artifact
  extraction, UKI + disk verification, and build metadata. The slot erofs
  is the split partition `dist/ingot_0.1.0.slot.raw` (the unit of
  reproducibility and the sysupdate transfer).
- **Harness:** `harness/run.sh` boots the deployed disk in QEMU (KVM,
  -cpu host) under pinned OVMF snakeoil with Secure Boot, asserts 10
  boot invariants from the serial console + guest probe, and writes
  `dist/harness/results.json` (exit code 0 = all pass).

## Acceptance evidence (2026-09-17)

- UKI: snakeoil signature verified; embeds .linux, .initrd, .ucode,
  .cmdline (root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec
  rootfstype=btrfs usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc
  usrfstype=erofs), .osrel. Verified at build time (build.sh).
- Disk layout: esp / ingot_0.1.0 / _empty / var / home, fixed PARTUUIDs
  from `image/mkosi.repart/` (verified at build time; boot-confirmed by
  the harness).
- Harness: all 10 assertions pass - uefi_secure_boot (kernel console
  "Secure boot enabled" + guest efivars SecureBoot entry),
  boot_from_uki, root_tmpfs, systemd_pid1, usr_slot_ro (erofs, ro,
  fixed slot PARTUUID), var_btrfs, etc_present (bind over /var/lib/etc,
  factory fstab materialized), multi_user_reached, journal_clean,
  brush_sh (/bin/sh -> /usr/bin/brush). Second boot of the same disk
  (reboot path) also passes: /var state survives, /etc never clobbered.
- Reproducibility: two independent `--local` builds from the archive
  produce a bit-identical slot erofs:
  sha256(ingot_0.1.0.slot.raw) =
  6f1bbe5a0ca9fa2ff75195a057b2d1df0a5a44a611b062d3b67d1d1fb3d11b5d
  (build 12 = build 13; deterministic via source_date_epoch pinned to
  the compose date, recorded in dist/build-metadata.json).

## The microcode path (verified against sources)

- Kernel 7.3-rc3 (rawhide) AMD microcode loader:
  - Early (pre-SMP): scans the initrd cpio for
    `kernel/x86/microcode/AuthenticAMD.bin` (container format; multiple
    concatenated containers are OK - `scan_containers`).
  - Late (firmware loader): requests
    `amd-ucode/microcode_amd_fam19h.bin` (per-family) with fallback to
    `amd-ucode/microcode_amd.bin`.
- The kernel does NOT parse UKI PE sections. The `.ucode` section is
  consumed by **systemd-stub**, which merges it ahead of the main initrd
  into the single initrd it hands to the kernel. .ucode must be an
  UNCOMPRESSED cpio. The kernel's `unpack_to_rootfs` walks leading cpio
  entries, then decompresses the remaining archive (init/initramfs.c).
- rawhide (fc46) moved the AMD microcode out of `linux-firmware` into a
  separate **`amd-ucode-firmware`** package, as a **directory** of
  container files under /usr/lib/firmware/amd-ucode (microcode_amd.bin
  for legacy families plus per-family microcode_amd_famXXh.bin). The
  slot's finalize concatenates them into the cpio's AuthenticAMD.bin.
- dracut 111 (rawhide) has NO microcode module - early microcode comes
  only from the UKI `.ucode` section.
- mkosi 26: `ukify --microcode` writes the `.ucode` section; files in
  `$ARTIFACTDIR/io.mkosi.microcode/` feed it. The UKI is built in the ESP
  image context (no firmware there), so the microcode initrd is staged
  explicitly from the slot image.
- The probe's `microcode_rev` is informational only: with `-cpu host`
  the guest starts at the host's revision, so a revision-diff
  assertion is not reliable; this kernel does not expose
  /sys/devices/system/cpu/microcode/revision in the guest (shows
  "unknown").

## Boot flow (verified by the harness, against the actual rawhide RPMs)

- dracut 111 + systemd 262: the `initrd-switch-root.service` /
  `initrd-cleanup.service` units are GONE from both RPMs; dracut's
  `11systemd-initrd` module installs them with `inst_multiple -o`
  (optional), creates `/etc/initrd-release`, and systemd 262 performs
  the switch-root to /sysroot BUILT-IN (the systemd binary carries the
  switch-root logic; `/etc/initrd-release` is the initramfs marker).
- `usr=`/`usrfstype=` on the kernel cmdline are ignored by stock dracut
  generators; only 99ingot consumes them via `getarg`.
- 99ingot (`image/files/usr/lib/dracut/modules.d/99ingot/`):
  ingot-root.service (After=sysroot.mount, Before=initrd-cleanup.service,
  WantedBy=initrd.target) runs prepare-root.sh: unmounts the state
  partition from /sysroot, mounts a tmpfs there, the slot erofs read-only
  at /sysroot/usr (by-partuuid from `usr=`), the state partition at
  /sysroot/var, materializes the factory /etc onto /var/lib/etc (first
  boot only, `cp -an`, never clobbers), binds /var/lib/etc over /sysroot/
  etc, creates the usr symlinks + empty top-level dirs, and verifies the
  systemd binary in the slot.
- The harness probe (image/files/usr/lib/ingot/harness-probe.sh,
  ingot-harness-probe.service After=multi-user.target) collects boot
  state as JSON: writes /var/lib/ingot/harness-probe.json and prints it
  between frame markers on the console in ONE write. It runs under
  /bin/sh (brush), POSIX-only.

## Harness parsing notes (learned the hard way)

The serial console frame is NOT a clean copy of the probe output:

- journald's ForwardToConsole rewrites each line through the kernel log
  prefix (`[    6.034954] sh[661]: ...`), one line per write, so console
  prefix stripping is mandatory in the harness parser.
- PID1's own console output (`[  OK  ] Finished ...`) and the serial
  getty's INTERACTIVE SHELL terminal-init sequence (DEC soft reset, OSC
  104, cursor save/restore, DSR query - a burst of ANSI/DEC escapes
  written to ttyS0 right at login) interleave into the byte stream,
  sometimes prefixing an individual frame line. The parser strips ANSI/
  DEC sequences (CSI incl. private markers, OSC with ST/BEL terminator,
  2-byte ESC functions), strips the log prefix, keeps only JSON-shaped
  lines, and json.loads-validates the reconstruction. A line that
  interleaves mid-JSON would make the parse fail loudly (empty probe,
  all checks fail) - visible, never silent.
- Secure Boot evidence: this guest kernel does NOT expose
  /sys/firmware/efi/sbs or loader-features (only efivars/, config_table/,
  fw_platform_size, fw_vendor, runtime, runtime-map, systab). The check
  is: kernel console line "secureboot: Secure boot enabled" AND, when the
  guest exposes /sys/firmware/efi/efivars, the SecureBoot-8be4df61-...
  variable entry present.

## Known risks / suspects for T2+

- The A/B flip, sysupdate transfer, and rollback flows are untested
  (later tickets). Slot identity = versioned GPT label ingot_<v>;
  `_empty` marks the free slot.
- mkosi 26 runs host-side prepare/finalize scripts in a bwrap sandbox
  that only sees the tools tree, the workspace, artifacts/packages, and
  BuildSources mounts (no host FS). `image/mkosi.conf` carries
  `BuildSources=..` so `$SRCDIR` (repo root at /work/src) reaches every
  script; `Environment=` does not propagate to subimages without
  `PassEnvironment=`.
- The ESP subimage needs the `kmod` package: mkosi runs depmod inside
  every image carrying /usr/lib/modules/<kver>.
- erofs determinism: repart/mkfs.erofs output is bit-stable for identical
  inputs (verified across two builds); source_date_epoch is pinned to
  the compose date so inode timestamps are stable. Do not rely on mtime
  from the host clock.

## Resume procedure (T2+)

1. `tools/build.sh --local` (~15-20 min; monitor the log for
   "build: done"). Expect the slot erofs sha to match
   dist/build-metadata.json for unchanged inputs.
2. `harness/run.sh` (1-2 min) must stay green after any image change.
3. Compose pin: if Rawhide moves past the archived compose, re-run
   `tools/archive-compose.sh` for the new pinned ID and update
   `tools/pins.json`.
