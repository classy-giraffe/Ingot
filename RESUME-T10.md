# T10 resume note (2026-09-18)

State of issue #21 (T10: admin userland: nushell, helix, zellij,
uutils). Read before starting T11+.

## Where things stand

**T10 complete.** All four acceptance criteria verified on a real
boot: the full image builds green (15m59s, UKI + disk layout
verified), the T1 harness passes 17/17 including the new
admin-userland checks, and the slot's erofs artifact carries all
four components (listed from the mounted artifact, below). Issue
#21 closed.

T10 developed on the `feat/t10-admin-userland` branch (6 commits)
from main's merge base (ced5397, post-T5).

Read RESUME-T1/T2/T3/T4/T5.md first (build chain, per-version
builds, A/B harness, engine conventions, wizard).

## What T10 added

- **The slot's admin userland (spec 7.2, criteria 25.9/25.10):**
  nushell (0.115.1, from T4), helix (25.07.1), zellij (v0.45.1),
  uutils coreutils (0.12.0) - all cargo-built from pinned upstream
  source, installed into the slot via $DESTDIR in
  `image/mkosi.build.chroot`. uutils binaries shadow the distro
  coreutils files in the final payload (the distro package stays
  installed: systemd, dracut, kernel-core, util-linux hard-require
  it).
- **Pinned-source model, in-repo manifest:** every Rust component
  is a git submodule of this repo (`thirdparty/<name>`, one per
  component) checked out at the upstream commit recorded in
  `tools/pins.json` (repo + release tag + commit SHA). This replaced
  the earlier tarball fetch for brush/nushell - one sourcing
  mechanism for all five components. `tools/check-pins.py` enforces
  each pin three ways: the mkosi.conf `Environment=<NAME>_SHA=`
  line, the `.gitmodules` URL, and the submodule's checked-out HEAD.
  After cloning: `git submodule update --init --depth 1` (README).
  Note: the brush pin was corrected from a2886cc (a pre-tag
  commit) to 96a26d0, the actual `brush-v0.4.0` tag commit.
- **Harness (probe + invariants + unit tests):** the probe
  (`harness/probe/harness-probe.sh`) runs from the booted slot:
  criterion 9 (`/bin/sh` resolves to `/usr/bin/brush`), criterion
  10 (`/usr/bin/nushell -c "print 'ok'"` runs headless), helix and
  zellij launch without error (`--version`). `harness/run.py`
  asserts them as `brush_sh`, `nushell_default_shell`,
  `admin_userland`; missing probe fields fail the check. New
  host-side unit tests (`harness/test_run.py`, the first
  run.py tests) pin the check behavior and the missing-field
  failure modes.
- **Helix grammar staging:** helix's build script compiles the
  tree-sitter grammars pinned in its own `languages.toml` (repo +
  revision per grammar). The prepare phase prefetched them with the
  exact loader operations (git init/remote/fetch --depth 1/checkout)
  so the chroot build reports GitUpToDate and needs no network.
  Recorded deviation: the gotmpl grammar's original repo was
  deleted upstream after the pin; the same commit is fetched from
  the `ngalaiko/tree-sitter-go-template` mirror (the prepare
  script logs it).
- **Build-environment fixes (the last blocker):** uutils'
  chcon/runcon link libselinux (package: `libselinux-devel`, not
  `selinux-devel`); selinux-sys generates bindings with bindgen,
  which drives libclang - rawhide's clang-libs layout (libclang in
  /usr/lib64, builtin headers in /usr/lib/clang/<major>/ via
  clang-resource-filesystem) does not satisfy libclang's
  resource-dir lookup from the load location, so the build script
  exports `BINDGEN_EXTRA_CLANG_ARGS=-resource-dir <dir>` (bindgen
  appends it to every clang invocation). Both are BuildPackages
  only - never in the payload.

## Verification evidence (2026-09-18)

1. Full build: `just build` green in 15m59s (prepare 4m, five
   cargo builds, payload stage, UKI signed, disk layout verified).
2. T1 harness (probe mode) 17/17 PASS: uefi_secure_boot,
   boot_from_uki, root_tmpfs, systemd_pid1, usr_slot_ro,
   var_btrfs, etc_present, multi_user_reached, journal_clean
   (24.1 shell-compat smoke: no failed units, no crit entries,
   boot still green under brush /bin/sh), brush_sh,
   nushell_default_shell, admin_userland, slot_version,
   esp_entry_present, esp_default, update_helper, probe_frame.
3. Slot contents (erofsfuse on dist/ingot_0.1.0.slot.raw):
   bin/nu + bin/nushell -> nu, bin/helix + bin/hx -> helix,
   bin/zellij, bin/brush + bin/sh -> brush, uutils (cat, dd, df,
   ls, mount, coreutils multi-call); 490 binaries in /usr/bin.
4. tests: 54 host-side (harness) + 90 Rust workspace (63+6+5+16),
   all green. `tools/check-pins.py` all green (compose, kernel,
   five userland pins x3, slot label, payload boundary).

## Review (two axes, 2026-09-18)

Ran inline (the review sub-agents hit a provider admission
limit, 3 strikes). Fixed point: main (ced5397..e390f57).

- **Standards:** no hard violations. Conventional commits +
  sign-offs; all files under the 500-line guardrail (largest:
  run.py 384); probe additions stay POSIX-only (no awk/sed/grep/
  eval); tests pin behavior and failure modes, not
  implementation. Judgement calls, left as-is: the per-component
  `cargo fetch` blocks in the prepare build phase and the
  per-component stage pairs repeat the same shape (a data-driven
  loop is possible; the explicit blocks match the T1/T4 pattern
  and keep per-component failure diagnostics); `grammar_mirror`
  carries a `*)` default - one recorded deviation today, an
  extension point for the next.
- **Spec (issue #21):** all four acceptance criteria met and
  verified (evidence above). No missing requirements. Bundled
  changes, all in service of criterion 1: the brush re-pin to the
  real tag commit, the tarball-to-submodule sourcing
  unification, the gotmpl grammar mirror (recorded deviation).
  Scope note: criterion 10 ("nushell is the default interactive
  shell") is asserted at image level - presence + headless run -
  because the login-shell wiring is the installer's job (the
  installer's default user shell references /usr/bin/nushell,
  per CONTEXT.md); T10's scope is the slot payload.

## Follow-up tickets

- T11+ per the parent spec (#14): the next workstation slice.
  The image side of the terminal experience (spec 7.2) is now
  complete: editor + multiplexer + coreutils + shells.
- If the helix pin moves: re-stage the grammar repositories
  (revisions are pinned by the helix source itself, no extra pin),
  and re-check the gotmpl mirror still carries the commit.
- If the rawhide clang layout changes (libclang moved out of
  /usr/lib64, or builtin headers moved out of /usr/lib/clang/):
  the build script's resource-dir glob
  (`/usr/lib/clang/*/include` with a stddef.h presence check)
  fails loudly with a clear message - re-derive the path then.
