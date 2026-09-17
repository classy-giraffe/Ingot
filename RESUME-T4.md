# T4 resume note (2026-09-17)

State of issue #18 (installer engine: declarative TOML). Read before
starting T5+.

## Where things stand

**T4 complete.** All acceptance criteria verified on a real qcow2
run; issue #18 closed. T4 developed directly on `main` (unlike
T1-T3 feature branches): the engine commits plus the code-review
fix batch and the standards/spec review follow-ups (parse split,
phase splits, preflights).

Read RESUME-T1/T2/T3.md first (build chain, per-version builds,
A/B harness).

## What T4 added

- **`ingot-installer` crate** (`src/installer/`, second workspace
  member, spec 7.1): binary `ingot-installer`. Parses and
  validates the TOML config (all spec 11.4 categories: disk,
  source, hostname, timezone, locale, keymap, partition sizing,
  filesystem choices, encryption [accepted, rejected at v1],
  initial users, SSH authorized keys, services) - one parse
  function per category, all diagnostics collected in one pass -
  then drives one engine through the phases: validate -> repart
  partitioning from the fixed-UUID definitions
  (`image/repart-baseline/`) -> mkfs -> whole-image slot
  deployment to both slots (active `ingot_<v>` + `_empty`) ->
  `/var/lib/etc` initialization (factory tree from the slot plus
  config: hostname, localtime, locale, keymap, users, SSH keys,
  service enables) -> ESP/UKI verification -> finalization
  (sync, unmount). Logs every action to `<work>/install.log` and
  copies it to the target's `/var/lib/ingot/install.log` on
  success and on failure.
- **Config is the authorization.** Non-interactive by design
  (spec 11.4): no prompts. `--dry-run` reports the full plan and
  touches nothing (verified byte-identical qcow2).
- **Working-copy model.** File targets (qcow2) are never modified
  in place: `losetup` partscan attach, `dd` whole-disk copy into
  the work directory (default `/run/ingot-install`, `--work` to
  override), repart on the copy, atomic rename back. A preflight
  checks the work volume holds the working copy (tmpfs `/run` can
  be smaller than the disk).
- **ESP handling.** The prebuilt ESP artifact carries
  systemd-boot and the UKI at `/EFI/Linux/ingot_<v>.efi` (the
  image's convention; systemd-boot >=250 auto-discovers
  `*/EFI/Linux`). The engine copies the whole ESP, verifies the
  UKI bytes against the artifact, and records `bootctl status` in
  the log. The boot phase is verification, not construction:
  bootctl install is baked into the prebuilt ESP artifact.
- **Tests.** 49 installer unit tests + 5 CLI contract tests
  (args, exit codes, stderr): config parse/validate diagnostics,
  layout computation (incl. the `mkfs_set` tool-set derivation),
  exact repart def rendering, capacity and tool-availability
  preflights, UKI validation. Test modules live next to their
  crates (`config/tests.rs`, `engine/tests.rs`, `layout/tests.rs`);
  parsing lives in `config/parse.rs` (one function per category).

## Preflights (fail before the first write)

- `--dry-run`: plan only; works on any host (no UEFI needed).
- UEFI mode active on the running machine (run path only; Ingot
  installs UEFI targets only).
- Required tools present in PATH (systemd-repart, dd, losetup,
  partprobe, qemu-img, blkid, mount, umount, sync, df, plus the
  layout's mkfs set - `layout::mkfs_set`, which skips the
  unformatted free slot) - 11.5 phase 1. `bootctl` is NOT in the
  hard set: its use is best-effort at runtime (the ESP artifact is
  prebuilt). The preflight previously demanded every partition's
  mkfs binary including `mkfs.unformatted` - a bug that blocked
  every real run; fixed in the second review round.
- Target is a qcow2 or raw file / block device (root for disk
  ops).
- Work volume free space >= disk virtual size.

## Verification evidence (installed disk, 2026-09-17)

1. Full layout with the fixed UUIDs: `esp` / `ingot_0.1.0` /
   `_empty` / `var` / `home` (1+8+8+4+8 GiB), labels standalone,
   PARTUUIDs embedded in the UKI cmdline.
2. Slot A: 8 GiB erofs of the `/usr` tree (os-release Ingot
   0.1.0, factory tree present); slot B: `_empty` unformatted.
3. `/var/lib/etc` initialized: factory defaults + hostname,
   localtime -> `/usr/share/zoneinfo/America/Chicago` (runtime
   path - the release ships no bare `CST` zone), locale, keymap,
   machine-id (fresh 32-hex random per install, 11.5 P6), user
   `tommy` (uid 1000, shell, shadow 0600, authorized_keys 600).
4. ESP: `EFI/BOOT/BOOTX64.EFI`, `EFI/systemd/...`,
   `EFI/Linux/ingot_0.1.0.efi` byte-exact vs the artifact.
5. `bootctl status` on the target shows systemd-boot on the
   loader; engine phase log in `/var/lib/ingot/install.log`
   (partprobe outcome logged, `target-warning` recorded).
6. `--dry-run` qcow2: sha256 identical before/after.
7. The 11.6.1 destructive warning is printed to stderr and logged
   before the first write (second review round).
8. Workspace tests: 49 installer + 5 CLI + 16 update-helper
   green.

## Resume procedure (T5+)

- `just build` (or `tools/build.sh --local`): green ~15-20 min;
  `just rust-test` after any src/ change (offline, seconds).
- Dry-run an install:
  `sudo src/target/release/ingot-installer --dry-run /path/config.toml`
  (`[source] base` in the config points at the artifact dir, e.g.
  the repo's `dist/`).
- Real run: same without `--dry-run`; add `--work /big/dir` when
  `/run` is too small for the disk's virtual size.
- The qcow2 target must be created first, and must hold the
  layout (29 GiB + GPT overhead): use 30G, not 29G - a 29G
  virtual disk fails the capacity preflight by 2 MiB:
  `qemu-img create -f qcow2 t4/disk.qcow2 30G`.

## Known gaps (image side, not installer)

- No `sshd.service` in the image (not in
  `/usr/lib/systemd/system`): the engine logs
  `service-skip` and continues - the spec allows enabling only
  shipped units. T5+ wants sshd in the image for the workstation
  profile.
- The image does not ship `nushell` even though the spec makes it
  the required default interactive shell (spec 3.1, 11.4, 20.1)
  and the config's default shell is `/usr/bin/nushell`. The
  release validation (shell must exist in the slot) correctly
  rejects configs that use it until the image installs it. The
  compose archive carries `nushell-0.99.1-8.fc46.x86_64.rpm`, so
  adding it to the mkosi `Packages=` list is a one-line image
  change (T5+). Verify installs with `shell = "/usr/bin/bash"`
  in the meantime.
- Factory tree has no base-system users
  (`/usr/lib/users/0001.toml` missing): `shadow`/`passwd`/`group`
  get only the configured users. Image build should add the base
  entries.
- The ESP artifact root carries the image build's
  `symvers-*.xz` files (harmless; image build cleanup).
- Repart defs pin slot A `Compression=zstd` (the image's slot
  artifact is zstd; ext4 defs carry no Compression key).

## Review follow-ups (open judgement calls)

- Error style: the installer uses flat `Result<_, String>`
  (operator-facing one-line diagnostics) instead of the
  update-helper's `anyhow::Context`. Consistent within the crate;
  a future change should pick one idiom workspace-wide.
- File-size guardrail (files under 500 lines): config parsing in
  `config/parse.rs`, test modules in per-crate `tests.rs` files.
  All installer product files land under the limit.
- `DiskTarget::prepare` builds the 7-field struct per target kind;
  a builder would be speculative until a fourth kind appears.

## Follow-up tickets

- #19 (T5: interactive TUI wizard, ratatui) is the next slice and
  builds on `ingot-installer`'s config/plan types; the wizard's
  output should feed the same TOML config as authorization.
- T6 (Secure Boot test path) will want the installer's boot phase
  to sign/verify against enrolled keys.
