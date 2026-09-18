# T5 resume note (2026-09-18)

State of issue #19 (interactive TUI wizard, ratatui). Read before
starting T6+.

## Where things stand

**T5 complete.** All acceptance criteria verified by the E2E harness
(harness/wizard.py, 20 checks) on a scratch qcow2: a full wizard
install and an engine-mode rerun with the wizard's written config
produce byte-identical partitions, slot A, and ESP; identical /var
and /home trees; identical install.log event sequences (27 events
each, same sequence, final "install-complete"). Abort at the
confirmation and at the first screen both exit 130 and leave the
target disk byte-identical; an early abort writes no config.

T5 developed on the `feat/t5-tui-wizard` branch from main's merge
base (74381aa), per the git conventions (feature branch, small
commits).

Read RESUME-T1/T2/T3/T4.md first (build chain, per-version builds,
A/B harness, engine conventions).

## What T5 added

- **`wizard` module** (`src/installer/src/wizard.rs` + `tests.rs`):
  the testable core. `Draft` holds the eleven spec-11.4 input
  categories as editable raw fields with workstation-profile
  defaults (hostname ingot, UTC, C.UTF-8, us, 1+8+8+4+8 GiB layout,
  erofs/btrfs/btrfs, unencrypted, one initial user). `Step` is the
  screen order (one per category, then the review). `Draft::config`
  is the single validation gate: render the fields to TOML, re-parse
  with the engine's strict parser (`config::parse`), so the wizard
  reports exactly the diagnostics the engine would. The wizard
  never installs on its own: it writes the config the engine already
  runs (spec 11.3 - no second install path).
- **`config::render`** (`config/render.rs`): canonical TOML
  rendering of a validated `Config` (the wizard's config writer).
- **`source::available_versions`**: scans a source base for
  `ingot_<v>.slot.raw` artifacts, newest first; feeds the wizard's
  default version choice (down-arrow on the source screen).
- **`--wizard` mode** (`cli.rs`, `main.rs`): `ingot-installer
  --wizard [--dry-run] [-w DIR] [--config PATH]`. Requires a TTY
  (non-interactive stdio fails with a usage error, exit 2).
  Exit codes: 0 install complete / dry-run plan reported; 130 user
  abort; 1 engine or config-write failure; 2 usage.
- **ratatui layer** (`wizard/ui/`): thin rendering + input over the
  state machine, split into `ui.rs` (run loop, screen state,
  review/plan/confirm transitions, 297 lines), `ui/ui_draw.rs`
  (drawing, 432 lines), `ui/ui_key.rs` (key input, 366 lines). All
  under the 500-line guardrail.
- **`harness/wizard.py`**: the pty E2E driver (drives the real TUI,
  real engine, real disks; identity checks via `mount -o ro,loop`
  partition access; scratch in `t5/`, workdir `src/target/`).

## Design decisions (locked during T5)

- **Single execution path (spec 11.3).** Wizard flow: collect ->
  review (strict-parse + engine-plan diagnostics gate) -> write
  config file -> `engine::plan` computed from the written file ->
  confirm screen (type 'yes', 11.6.2) -> `engine::run` on that
  file. The confirmation screen shows the engine's own plan
  report.
- **The review gate is the engine's validation.** It runs the
  strict parser *and* the engine's read-only plan, so the user sees
  the engine's own rejections (luks2 in v1, an undersized disk, a
  missing source artifact) at the review - before any file is
  written - not at the plan.
- **Confirmation is explicit everywhere.** Esc on any step screen
  prompts once (y/n) before aborting; esc on the confirmation
  screen also prompts (it was a silent exit - the abort must be a
  deliberate act). Abort = exit 130, target untouched (the engine's
  working-copy model guarantees untouched-on-failure as well). The
  wizard re-writes its own config file when the user goes plan ->
  review -> plan again; a file the user pointed --config at is
  never overwritten.
- **Key input: arrows + Insert/Delete only.** j/k/a/x were removed:
  they intercept printable characters that belong to field input
  (typing a hostname with k would be eaten as movement). Row
  navigation is the arrow keys; row actions are Insert (add) and
  Delete (remove); the screen hints state exactly that. Left/right
  on the Users screen switches the name/shell sub-field; left/right
  on Filesystems/Encryption toggle the two v1 choices.
- **Version auto-discovery.** If the base directory exists at
  startup, the first version found is pre-filled on the source
  screen (down-arrow to cycle); otherwise empty.
- **Encryption: accepted, rejected at v1.** The category is
  collected (11.4.8) and luks2 can be selected; the engine's
  preflight rejects it with a clear diagnostic (LUKS2 deferred -
  spec MAY, spec 7/11.4.8).
- **Shell model (unchanged from T4, recorded in CONTEXT.md):**
  initial users get /usr/bin/nushell when the installed release
  pins one (the validation phase checks it against the slot);
  otherwise the engine default /usr/bin/bash; an explicit shell in
  the config wins.

## Bugs found and fixed during T5 review

- `cycle` set the choice from the key direction, so from the
  default value the right arrow was a no-op (btrfs -> Btrfs) and
  the encryption cycle was a no-op in all four directions. Left and
  right now both toggle (a6a6476).
- j/k/a/x key interception and the silent esc-at-confirmation
  (7ced127).
- `add_row` panicked on an empty row list: the Ssh and Services
  screens start empty, and inserting at sel + 1 past the end of the
  vec crashed the TUI (skipping the terminal restore). The insert
  index is clamped to the list length (c9f0597, with headless
  key-handler regression tests in ui/ui_key.rs).
- After any plan failure, going back to the review and advancing
  again dead-ended on "config already exists" - the wizard's own
  written file blocked the fix loop. The wizard now re-writes its
  own config on later review passes; a user-supplied --config file
  still refuses the overwrite (269e6f7).
- The review gate ran only the strict parser, so a config the
  engine would refuse (luks2 in v1, undersized disk, missing
  artifact) passed review with "no errors" and was only rejected
  at the plan. The review now also runs the engine's read-only plan
  and shows its diagnostics (ea5997f).
- Standards-axis cleanups: append/backspace collapsed onto one
  field() selector, step_errors runs the gate once, the install-log
  path has one accessor (ba3a8d4).
- pty escape-sequence pairing in the E2E driver (arrow keys and
  esc must not be split or run together across writes) - harness
  side, not product.
- install.log lives at /var/lib/ingot/install.log on the installed
  system (the var partition mounts at /var), not /var/install.log
  - harness-side path fix.

## Verification

- Unit: 69 tests (63 crate + 6 cli) green, including the wizard
  state-machine tests (navigation, field editing, row add/remove,
  review gate, abort, dry-run), the headless key-handler tests
  (insert on empty lists, insert-after-selected), and the config
  render round-trip (render -> strict parse -> identical config).
- E2E: harness/wizard.py 20/20, re-run after every fix batch.
- Manual pty smoke: version auto-fill, cycling to ext4 (written
  config carries fs var = "ext4"), luks2 rejected at the review
  gate with the engine's own diagnostic, the plan -> review -> plan
  rewrite loop, and the abort paths.

## Conventions kept

- Per-crate `tests.rs` test modules; files under 500 lines
  (guardrail); `anyhow::Context` within the installer crate;
  exit codes 0/1/2 (+130 SIGTERM abort convention); one parse
  function per config category (the wizard reuses them via
  `Draft::config`); diagnostics are collected, not fail-fast.

## Follow-up tickets

- T6 (Secure Boot test path) hooks into the engine's boot phase
  (`src/installer/src/boot.rs`: bootctl install + UKI placement):
  sign/verify against enrolled keys there. The wizard needs no
  change - it is a front-end over the same engine.
- LUKS2 (deferred) would land in the engine's repart/mkfs phases
  (the partition-encryption definitions in `image/repart-baseline/`);
  the wizard already collects the category.
