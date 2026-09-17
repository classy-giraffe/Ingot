# T3 resume note (2026-09-17)

State of issue #17 (update helper crate: discovery, auth, URL pinning).
Read before starting T4+.

## Where things stand

**T3 complete.** All acceptance criteria verified; issue #17 closed.
Commits on `feat/t3-update-helper` (branch is main + these two commits;
not merged): `b51a7a3` (crate), `4fea035` (image integration).

Read RESUME-T1.md and RESUME-T2.md first (build chain, per-version
builds, A/B harness).

## What T3 added

- **In-repo Rust workspace** (`src/`, spec 7.1): member
  `update-helper`, binary `ingot-update-helper`. `Cargo.lock` is
  committed - the crate versions are part of the build pin.
- **The helper** (narrow, spec 22): `--repo OWNER/REPO --keyring PATH
  [--api URL] [--asset-base URL] [--out PATH]`. Discovers the latest
  release (highest semver non-prerelease tag from the Releases API;
  the date-based latest is not used), authenticates it (manifest.json
  and SHA256SUMS GPG signatures verified against the keyring via
  sequoia-openpgp; SHA256SUMS cross-checked against the manifest), and
  emits the pin document: `{schema, repo, tag, version, asset_base,
  assets: [{name, sha256, url}]}` - the shape the sysupdate transfers
  consume (asset_base = `[Source] Path=`, name = `MatchPattern=`
  expansion, version = `systemd-sysupdate update <v>`; mapping
  documented in `src/update-helper/src/pin.rs`).
- **Fixture-driven tests** (`src/update-helper/tests/`, 21 green): the
  Releases API and asset directory sit behind a `file://` URL scheme,
  so the tests are hermetic (no network). Fixture repos: basic,
  datetrap, multidigit, prerelease, prerelease-only, empty, tampered,
  tampered-sums, unknownkey, incoherent, sums-missing, missing-manifest.
  Run with `just rust-test`.
- **Image build phase**: `image/mkosi.prepare` (final phase) stages
  `src/` into the artifacts dir (mkosi mounts it from
  `BuildSources=../src:ingot-src`); `image/mkosi.build.chroot` builds
  it `--locked --offline` against the primed registry and installs it
  to `/usr/bin/ingot-update-helper` (next to brush).
- **T1 harness invariant 10** (`update_helper`): the probe runs
  `/usr/bin/ingot-update-helper --help` in the guest; `harness/run.py`
  asserts it. 14 invariants total now.
- **justfile**: `just rust-test` (host-side cargo tests).

## Decisions and gotchas (do not rediscover these)

- **nettle3.10, not nettle 4.0.** The image carries `nettle3.10`
  (runtime) + `nettle3.10-devel` (build) + `gmp` (runtime NEEDED of
  libhogweed). Nettle 4.0 (also in the pinned compose) changed
  `nettle_hash_digest_func` to a 2-argument form; the nettle crate
  7.5 (latest) does not compile against it. Both nettle 3.10 and 4.0
  packages coexist in the compose; only ONE devel may be installed
  (both ship `nettle.pc`).
- **Pregenerated nettle bindings**
  (`src/update-helper/build/nettle-bindings.rs`): nettle-sys normally
  generates its bindings with bindgen, which needs libclang - the
  build chroot carries none. The committed file is passed as
  `NETTLE_PREGENERATED_BINDINGS` (image/mkosi.build.chroot);
  nettle-sys copies it verbatim and skips bindgen. It was generated
  from the pinned nettle3.10-devel headers with nettle-sys 2.3.2's
  exact build.rs configuration (bindgen 0.72.1). Regenerate only when
  the compose pin moves nettle3.10 (procedure in the file's header).
  Build packages `rust, cargo, gcc` are sufficient - no clang.
- **pkg-config in the chroot**: Rawhide's `/usr/bin/pkg-config` is a
  wrapper that shells out to `rpm` (absent from the slot image). The
  build script exports `PKG_CONFIG=/usr/bin/x86_64-redhat-linux-gnu-pkg-config`
  (the platform wrapper, which only execs pkgconf). Build packages
  include `pkgconf` and `pkgconf-pkg-config`.
- **Host-side note**: the host's cargo tests run against the host's
  nettle (Debian 3.x) - the 3-argument digest API, same as the
  pinned nettle3.10. That is why the host build works while a
  nettle-4.0 chroot would not.

## Resume procedure (T4+)

1. `just rust-test` (seconds) after any `src/` change; the 21 tests
   must stay green and offline.
2. `just build` must stay green; the per-version chain
   (`--version <v> --slot <a|b>`) unchanged.
3. `just harness` (now 14 invariants) and `just ab` must stay green
   after any image or harness change.
4. T4+ work (installer crate, admin-initiated update operation,
   sysupdate transfer integration) builds on this: the installer is
   the second workspace member (spec 7.1); the helper's pin document
   is its input for the release source.
