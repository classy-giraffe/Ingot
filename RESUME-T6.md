# T6 resume note (2026-09-18)

State of issue #23 (T6: Live ISO and unattended install end-to-end). Read
before starting T7+.

## Where things stand

**T6 complete.** All acceptance criteria verified by the E2E harness
(`harness/iso.py`, 14 checks across two scenarios) on real QEMU boots
with Secure Boot enabled (OVMF snakeoil keys enrolled):

- **Scenario A (Criterion 12, Live ISO boot):**
  - Boots headless with Secure Boot on to serial console with automatic
    root login directly into **Nushell 0.115.1** (`ingot-live login: root`).
  - Ephemeral state model verified: root `/` is tmpfs, `/usr` is read-only
    erofs from the ISO payload (`LiveOS/rootfs.erofs`), `/var` is tmpfs,
    `/etc` is bound over `/var/lib/etc` initialized from factory defaults.
  - State reset verified across a reboot: machine-id randomizes per boot
    (different across boots), and a scratch marker in `/var/tmp` disappears
    on reboot.
  - Reachable SSH: `sshd` starts automatically, accepts the live test key
    (`tools/keys/live.key` via `root/.ssh/authorized_keys`), and opens an
    interactive Nushell session.
  - Clean console output: `nowatchdog` eliminates CPU watchdog timeout
    messages while preserving early-boot Secure Boot verification.

- **Scenario B (Criterion 11, Unattended install):**
  - Auto-trigger: boots ISO with a declarative config disk (`config.raw`
    carrying `ingot-install.toml`) attached as `/dev/vdb` and blank target
    attached as `/dev/vda`. `ingot-live-install.service` detects the config
    disk and launches `/usr/bin/ingot-installer` unattended without human
    input.
  - Installer completes with exit code 0 (`INGOT-LIVE-INSTALL: complete`).
  - Host-side layout forensics verify the 5-partition A/B layout (`esp`,
    `ingot_0.1.0`, `_empty`, `var`, `home`), fixed PARTUUIDs, and ESP
    entries (`ingot_0.1.0` default and good state).
  - Reboot into installed machine: boots from `target.raw` with Secure Boot
    on, full A/B layout, and reaches `multi-user.target`.
  - Materialization of `/etc` verified: hostname set to `ingot-harness`,
    initial user `harness` created with home directory, authorized keys,
    and login shell `/usr/bin/nushell`.
  - Reachable SSH: `ssh harness@installed` connects and executes commands.
  - Clean boot: no initramfs failure markers on the console.

T6 developed on the `feat/t6-live-iso` branch.

Read RESUME-T1/T2/T3/T4/T5/T10.md first (build chain, per-version builds,
A/B harness, engine conventions, wizard, admin userland).

## What T6 added

- **Live ISO image builder (`tools/build-iso.sh`):**
  - Output: `dist/ingot_0.1.0.iso` (spec 10.3 layout).
  - Live UKI PE surgery: swaps the `.cmdline` section of `dist/ingot_0.1.0.efi`
    in-place with `root=tmpfs ingot.live console=tty0 console=ttyS0 nowatchdog`,
    truncates the stale signature, recomputes the Authenticode image hash, clones
    the dist PKCS#7 (updating `spcIndirectData` hash and `messageDigest`), re-signs
    with the snakeoil RSA key, and verifies with `sbverify`.
  - All sections except `.cmdline` remain byte-identical to the installed UKI.
  - Live ESP image: creates a FAT filesystem carrying the signed fallback
    loader (`BOOTX64.EFI`), the live UKI, and loader configuration.
  - Payload: extracts and trims the slot erofs filesystem from `slot.raw`.
  - Installed ESP tree (`esp/` on the ISO): exports the installed machine's
    ESP tree for live source deployment.
  - ISO assembly: hybrid bootable ISO9660 filesystem via `xorriso` with
    El Torito no-emulation EFI boot image and GPT partition table.

- **Initramfs live boot support (`image/files/usr/lib/dracut/modules.d/99ingot/`):**
  - `parse-live-root.sh`: dracut cmdline hook setting `rootok=1` so stock
    dracut command-line validation accepts the live boot arguments.
  - `ingot-live-root.service`: triggered by `ConditionKernelCommandLine`
    (`root=tmpfs` or `ingot.live`), ordered after `initrd-root-fs.target`
    and before `initrd-cleanup.service`.
  - `prepare-live-root.sh`: discovers ISO media by label `INGOTLIVE` (with
    fallback device scan), mounts `/sysroot/media/ingot-iso`, mounts
    `LiveOS/rootfs.erofs` at `/sysroot/usr`, materializes factory defaults to
    `/sysroot/var/lib/etc`, binds over `/sysroot/etc`, unlocks the console
    root password, sets root's shell to `/usr/bin/nushell` in `passwd`, installs
    autologin systemd drop-ins for `getty@tty1` and `serial-getty@ttyS0`, deploys
    the live SSH rescue key to `/sysroot/root/.ssh`, generates a per-boot
    `machine-id`, arms `ingot-live-install.service`, and creates runtime root symlinks.
  - `99-ingot.conf`: adds `iso9660` module to initramfs autoloading.

- **Workstation profile package baseline and factory defaults:**
  - Packages in `image/mkosi.conf`: `pam`, `openssh-server`, `authselect`,
    `systemd-networkd`, `erofs-utils`, `btrfs-progs`, `dosfstools`.
  - Factory defaults in `image/mkosi.postinst.chroot`: enables `sshd.service`,
    `systemd-networkd.service`, `getty@tty1.service`, `getty@ttyS0.service`,
    and `ingot-ssh-hostkeys.service`.
  - Sets root default shell to `/usr/bin/nushell` in `$factory_etc/passwd` and
    registers `/usr/bin/nu` and `/usr/bin/nushell` in `/etc/shells`.
  - Masked `systemd-loop@.service` to prevent failure on missing `systemd-dissect`.
  - Runs `authselect select local --force` and preserves `/etc/authselect`,
    `/etc/security`, and `/etc/pam.d` in `/usr/share/factory/etc`.
  - Default DHCP network configuration: `20-ethernet.network`.
  - On-boot host key generation: `ingot-ssh-hostkeys.service`.

- **Installer live source deployment mode & TUI redesign (`src/installer/`):**
  - `source.rs`: `LiveSource` struct and `resolve_live()` to resolve the
    running release's erofs payload and installed ESP tree from `/media/ingot-iso`.
  - `config/render.rs`: added `SourceMode::Live` serialization (emits `mode = "live"`
    without `version` key), fixing the review screen's `artifact not found` failure.
  - `target.rs`: auto-probes `/sys/block` to discover available writable and
    read-only block devices with human-readable sizes (`size_human()`).
  - `wizard/`: modularized drawing into `ui_draw.rs` (banner, breadcrumb ribbon,
    rounded cards), `ui_steps.rs` (forms with padded `LABEL_W = 18` preventing
    label collisions), and `ui_review.rs` (3-card executive summary).
  - `deploy.rs`: `deploy_live()` to deploy the slot erofs to slot A and copy
    the installed ESP tree to the mounted ESP partition.
  - `live-install.sh`: unattended installer driver for the live ISO, using `blkid`
    to skip raw unformatted disks before mounting.

- **Testing infrastructure & tooling:**
  - Integrated `uv` virtualenv at `.venv` with `qemu.qmp`, `virt-firmware`,
    `pefile`, `pytest`, `pytest-xdist`, and `ruff`.
  - `harness/run.py`: added QMP socket support with `qmp_powerdown()` and HMP fallback.
  - `harness/esp.py`: added `read_nvram_vars()` using `virt.firmware` to parse
    EDK2 NVRAM variable stores (`.fd`).
  - `harness/bootstate.py`: used `pefile` for structured `.osrel` extraction with
    manual struct fallback.
  - `justfile`: `just test` uses `pytest` when available, added `just lint` and
    `just fmt` via `ruff`.

## Verification results

```text
iso: scenario A (live ISO, criterion 12) on ingot_0.1.0.iso
  PASS  live_ssh  (ssh root@live -> hostname 'ingot-live' (rc 0))
  PASS  live_secure_boot  (Secure boot enabled (console))
  PASS  live_getty  (Headless serial terminal reached (console))
  PASS  live_root  (/ tmpfs, /usr erofs)
  PASS  live_ephemeral  (machine-id 05d1ed3f.. -> 39e07ed0..; marker gone)
iso: scenario B (unattended install, criterion 11)
  PASS  install_auto_trigger  (config disk found; unattended install started)
  PASS  install_complete  (installer exit 0 (marker on the serial console))
  PASS  installed_layout  (labels ['_empty', 'esp', 'home', 'ingot_0.1.0', 'var'])
  PASS  installed_esp  (entries ['ingot_0.1.0'] default 'ingot_0.1.0')
  PASS  installed_ssh  (ssh harness@installed -> hostname 'ingot-harness' (rc 0))
  PASS  installed_hostname  (/etc/hostname 'ingot-harness' (config materialization))
  PASS  installed_initial_user  (uid=1000(harness) gid=1000(harness) groups=1000(harness) | 1 | )
  PASS  installed_secure_boot  (Secure boot enabled (console))
  PASS  installed_no_failure_markers  (no initramfs failure markers on the console)
results: /home/tommy/Ingot/dist/harness/iso/results.json
```

- **Full harness test suite (`just test`):** 56/56 passed (23.5s).
- **Full Rust workspace test suite (`just rust-test`):** 103/103 passed (4 suites).
- **End-to-end install proof:** live ISO boots to Nushell in 13.2s, dry-run plan verified, install executes to `/dev/vda` with all 22 events logged (`install-complete`), and target boots independently with Secure Boot on into `ingot-workstation` in 8.0s.

## Follow-up tickets

- T7+: Next workstation slices (sysupdate update path, rollback testing in
  harness, and further workstation profile services).
