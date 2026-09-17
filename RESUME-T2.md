# T2 resume note (2026-09-17)

State of issue #16 (A/B selection and automatic rollback). Read before
starting T3+.

## Where things stand

**T2 complete.** All acceptance criteria verified; issue #16 closed.

Read RESUME-T1.md first (T1 ground state: build chain, harness, pin
procedures).

## What T2 added

- **Per-version builds.** `tools/build.sh --version <v> --slot <a|b>`
  builds the given slot partition with a per-version slot erofs
  (`dist/ingot_<v>.slot.raw`), per-version disk (`dist/ingot_<v>.raw`),
  and a versioned UKI entry (`ingot_<v>.efi`, os-release carries the
  version). The v0.1.0 chain is byte-identical to T1's.
- **A/B harness (`harness/run_ab.py`, `just ab`).** Five boots on one
  working disk:
  1. deploy: copy `dist/ingot_0.1.0.raw`, write the v0.2.0 slot erofs
     into the `_empty` partition (label `ingot_0.2.0`), install the
     armed `ingot_0.2.0+3` UKI (tries-left scheme, spec 12.2) next to
     the good `ingot_0.1.0` entry.
  2. boot 1 (v0.2.0): the armed entry is the newest non-bad entry, so
     the loader selects it by default; reaching multi-user blesses it -
     `systemd-bless-boot` renames the entry to the plain good name and
     the `selection` phase shows the counter gone (blessed).
  3. boots 2-4 (failure injection): re-arm the entry to `+3` and zero
     1 MiB of slot B's erofs (corrupts the superblock). Each failed boot
     shows the console-native marker (`Failed to start
     ingot-root.service`), reaches no multi-user, and the loader
     decrements the armed entry: `+3 -> +2-1 -> +1-2 -> +0-3`.
  4. boot 5 (automatic fallback): the exhausted entry (`+0`) is bad, so
     the loader falls back to the preserved v0.1.0, which boots to
     multi-user.
  Evidence per boot: `dist/harness/ab/results/<boot>/` (console, probe
  JSON, results.json) plus `dist/harness/ab/ab-results.json` (machine-
  readable: per-boot checks, per-phase ESP selection state - entries,
  counters, default entry - slot erofs sha, poweroff reason).

## First-boot admin layer (experiment-driven, T2)

The fresh-disk first boot hits systemd's first-boot flow ("Detected
first boot"): the preset policy is applied to all vendor units (the
catch-all `disable *` strips any admin enablement not listed in a
system preset) and the interactive `systemd-firstboot` wizard runs.
Two facts that bit T2:

- The image ships factory /etc defaults in `/usr/share/factory/etc`
  (the `systemd-firstboot.service` and `authselect-apply-changes.service`
  masks, `fstab`, journald/networkd config). The 99ingot dracut module
  (`prepare-root.sh`) materializes them onto `/var/lib/etc` (the /etc
  bind source) **per file, never clobbering**, on every boot. If that
  step is skipped, the interactive firstboot blocks the serial console
  forever (it waits for a timezone answer) and the boot is lost.
- The preset policy strips the harness probe's admin enablement on the
  scenario disk's first boot (the probe is harness-owned, injected into
  /var, not preset-listed). The probe injection therefore seeds a real
  machine-id (`/var/lib/etc/machine-id`, fixed fixture ID, set once) so
  the scenario disk's first boot is not a "first boot": no preset
  policy, no firstboot wizard, the probe's enablement survives.

Do not re-add the probe unit or its enablement to the image payload
(payload boundary), and do not make the factory materialization
all-or-nothing (`[ ! -e /var/lib/etc ]`): the injection pre-creates
`/var/lib/etc`, which then suppressed the materialization and broke the
first boot (the T2 incident: the guest sat at the "Please enter the new
timezone" prompt, and the preset policy had stripped the probe's
enablement).

## Boot-layer A/B mechanics (verified against systemd v262 source and
the live guest)

- **Entries**: UKI names under `EFI/Linux/`, versioned per spec 12.2.
  Counter state is encoded in the file name: `ingot_<v>.efi` (good),
  `ingot_<v>+<N>.efi` (armed, tries-left N, no attempts),
  `ingot_<v>+<N>-<M>.efi` (armed, decremented M times).
- **Counters**: the loader maintains tries-left per entry (man page:
  decremented on each boot; entry becomes *bad* at `tries-left=0`,
  *good* again after a successful boot or after the entry is re-armed).
  Bad entries are not selected unless all entries are bad.
- **Selection (default entry)**: bad entries (tries-left exhausted)
  are excluded from selection and only chosen if every entry is bad;
  among the rest the newest version wins (versionsort). So the armed
  v0.2.0 entry is selected over the good v0.1.0 entry until v0.2.0 is
  exhausted, when the default flips to v0.1.0 (verified live: the
  fallback boot).
- **Blessing**: `systemd-bless-boot` (from the `systemd-udev` package;
  the unit + generator ship in the image factory defaults) runs at
  boot-complete when the loader set `LoaderBootCountPath` for the
  booted entry; it renames the entry file to the plain good name. The
  harness settles 5 s after the boot marker before poweroff so the
  rename lands on the ESP.
- **Default entry**: the harness models it (newest non-bad entry, bad
  only if all are bad) and records it per phase in
  `ab-results.json` alongside the raw ESP entries.

## Failure-injection mechanics (experiment-driven)

Zeroing 1 MiB of the slot partition corrupts the erofs superblock
(offset 1024). The console-native evidence of the failure is the
manager's status line `[FAILED] Failed to start ingot-root.service`
(ANSI color-coded; the harness matches the ANSI/kernel-prefix-stripped
form via `console.console_contains`). The initramfs's own journal does
NOT forward service stderr to the console (Storage=volatile, no
ForwardToConsole), so do not rely on `ingot-prepare:` script messages
appearing on the serial console. Failure boots end in the initramfs
emergency shell; the harness powers them off via ACPI.

## Verification evidence (2026-09-17)

- `just test`: 47 unit tests OK.
- `just harness` (T1 gate, probe mode) and `just prod` (T1 gate, no
  probe): 14/14, green after the T2 changes (regression gate).
- `just ab`: all five boots pass; `ab-results.json` shows the bless
  rename, the `+3 -> +2-1 -> +1-2 -> +0-3` decrement chain, and the
  v0.1.0 fallback boot with v0.1.0's slot erofs sha unchanged.

Both images are rebuilt after any `prepare-root.sh` change (the module
is in the slot payload and lands in the UKI initramfs); the slot erofs
sha changes accordingly.

## Session-2 fixes (2026-09-17, recorded so T3+ does not regress them)

- **D-Bus vendor enablement** (`image/mkosi.postinst.chroot`): the
  scenario disk's seeded machine-id skips the first-boot preset flow,
  so the stock first-boot wiring of `dbus.service` (the broker alias)
  and `sockets.target.wants/dbus.socket` never materializes; without
  them `dbus.socket` cannot load its target and systemd-logind fails
  in cascade (dirty journal). The image now bakes both as vendor
  symlinks under `/usr/lib/systemd/system/` (the /etc bind hides
  /etc enablement at boot).
- **Harness hardening**: `esp.py` mtools calls never inherit the
  caller's stdin or controlling TTY (`stdin=DEVNULL` +
  `start_new_session`); mtools opens /dev/tty itself for overwrite
  prompts, and a pre-existing mcopy destination otherwise hangs the
  harness on a live PTY. `console.py` strips ANSI/terminal sequences
  before line filtering (a JSON line colored or prefixed mid-line is
  not dropped). `run_ab.region_sha` hashes exactly the partition's
  LBA range. The A/B `secure_boot` check uses the T1 gate's
  efivars-based fallback (the guest kernel exposes no
  /sys/firmware/efi/sbs; the SecureBoot efivar is the guest-side
  evidence).

## Resume procedure (T3+)

1. `tools/build.sh --local` must stay green; the v0.1.0 slot sha must
   match `dist/build-metadata.json` (regression gate).
2. `just harness` and `just ab` must stay green after any image or
   harness change.
3. T3 (config model, section 14) and T4+ build on top of the per-
   version build chain: keep `--version`/`--slot` semantics; the slot
   erofs (`dist/ingot_<v>.slot.raw`) remains the unit of
   reproducibility and of the sysupdate transfer.
