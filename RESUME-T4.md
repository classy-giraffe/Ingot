# T4 resume note (2026-09-17)

State of issue #18 (installer engine: declarative TOML). Read before
starting T5+.

## Where things stand

**T4 complete.** All acceptance criteria verified on a real qcow2
run; issue #18 closed. T4 developed directly on `main` (unlike
T1-T3 feature branches): the engine commits plus the code-review
fix batch (6 commits, 8ffa528..4c6d636) and `21470b8` (docs).

Read RESUME-T1/T2/T3.md first (build chain, per-version builds,
A/B harness).

## What T4 added

- **`ingot-installer` crate** (`src/installer/`, second workspace
  member, spec 7.1): binary `ingot-installer`. Parses and validates
  the TOML config (all spec 11.4 categories: disk, source,
  hostname, timezone, keemap/locale, partitioning, filesystem,
  encryption [accepted, rejected at v1], initial users, SSH
  authorized keys, services), then drives one engine through the
  phases: validate -> repart partitioning from fixed-UUID
  definitions (`image/repart-baseline/`) -> mkfs -> whole-image
  slot deployment to both slots (active `ingot_<v>` + `_empty`) ->
  `/var/lib/etc` initialization (factory tree from the slot plus
  config: hostname, localtime, locale, users, SSH keys, service
  enables) -> bootctl install + UKI verification -> finalization
  (sync, unmount). Logs every action to the target's
  `/var/log/install.log`.
- **Config is the authorization.** Non-interactive by design
  (spec 11.4): no prompts. `--dry-run` reports the full plan and
  touches nothing (verified byte-identical qcow2).
- **Working-copy model.** File targets (qcow2) are never modified
  in place: `qemu-nbd` attach, `dd` whole-disk copy into the work
  directory (default `/run/ingot-installer`, `--work` to override),
  repart on the copy, atomic rename back. A preflight checks the
  work volume holds the working copy (tmpfs `/run` can be smaller
  than the disk).
- **ESP handling.** The prebuilt ESP artifact carries systemd-boot
  and the UKI at `/EFI/Linux/ingot_<v>.efi` (the image's
  convention; systemd-boot >=250 auto-discovers `*/EFI/Linux`).
  The engine copies the whole ESP, verifies the UKI bytes
  against the artifact, and records `bootctl status` in the log.
- **Tests.** 45 installer unit tests + 5 CLI contract tests
  (args, exit codes, stderr): config parse/validate diagnostics,
  layout computation, exact repart def rendering, capacity
  preflight, UKI validation.

## Preflights (fail before the first write)

- `--dry-run`: plan only.
- UEFI mode active on the running machine (Ingot installs UEFI
  targets only).
- Target is a qcow2 or raw file / block device; qcow2 needs
  qemu-nbd (root).
- Work volume free space >= disk virtual size.

## Verification evidence (installed disk, 2026-09-17)

1. Full layout with the fixed UUIDs: `esp` / `ingot_0.1.0` /
   `_empty` / `var` / `home` (1+8+8+4+8 GiB), labels standalone,
   PARTUUIDs embedded in the UKI cmdline.
2. Slot A: 8 GiB erofs of the `/usr` tree (os-release Ingot
   0.1.0, factory tree present); slot B: `_empty` unformatted.
3. `/var/lib/etc` initialized: factory defaults + hostname,
   localtime -> `/usr/share/zoneinfo/CST` (runtime path), locale,
   user `tommy` (uid 1000, shell, shadow 0600, authorized_keys
   600).
4. ESP: `EFI/BOOT/BOOTx64.EFI`, `EFI/systemd/...`,
   `EFI/Linux/ingot_0.1.0.efi` byte-exact vs the artifact.
5. `bootctl status` on the target shows systemd-boot on the
   loader; engine phase log in `/var/log/install.log`.
6. `--dry-run` qcow2: sha256 identical before/after.
7. Workspace tests: 45 installer + 5 CLI + 16 update-helper green.

## Resume procedure (T5+)

- `just build` (or `tools/build.sh --local`): green ~15-20 min;
  `just rust-test` after any src/ change (offline, seconds).
- Dry-run an install:
  `sudo src/target/release/ingot-installer --dry-run /path/config.toml`
  (point `--artifacts-dir` at `dist/`; `[source]` in the config
  names the split artifacts).
- Real run: same without `--dry-run`; add `--work /big/dir` when
  `/run` is too small for the disk's virtual size.
- The qcow2 target must be created first:
  `qemu-img create -f qcow2 t4/disk.qcow2 29G`.

## Known gaps (image side, not installer)

- No `sshd.service` in the image (not in
  `/usr/lib/systemd/system`): the engine logs
  `service-skip` and continues - the spec allows enabling only
  shipped units. T5+ wants sshd in the image for the workstation
  profile.
- Factory tree has no base-system users
  (`/usr/lib/users/0001.toml` missing): `shadow`/`passwd`/`group`
  get only the configured users. Image build should add the base
  entries.
- The ESP artifact root carries the image build's
  `symvers-*.xz` files (harmless; image build cleanup).
- Repart defs pin slot A `Compression=zstd` (the image's slot
  artifact is zstd; ext4 defs carry no Compression key).

## Follow-up tickets

- #19 (T5: interactive TUI wizard, ratatui) is the next slice and
  builds on `ingot-installer`'s config/plan types; the wizard's
  output should feed the same TOML config as authorization.
- T6 (Secure Boot test path) will want the installer's boot phase
  to sign/verify against enrolled keys.
