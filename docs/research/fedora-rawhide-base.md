# Research: Fedora Rawhide base feasibility

**Date:** 2026-09-16
**Issue:** [classy-giraffe/Ingot#11](https://github.com/classy-giraffe/Ingot/issues/11)

Verifies the feasibility of the proposed Fedora Rawhide base for Ingot's mkosi build
of an immutable x86_64 UEFI Linux OS, against primary sources: the Fedora/Koji
package repository (the actual rawhide repo and its RPMs), the Fedora systemd/glibc
packaging specs on src.fedoraproject.org, the mkosi source tree and man page
(`systemd/mkosi`, `main`), the dracut source tree, the brush project repository,
the systemd man pages, and the Rust Cargo book.

**Method note:** all package versions below were read from the live rawhide
repository on `kojipkgs.fedoraproject.org` (Koji's primary package host) on
2026-09-16: the repo's `rpmlist.jsonl` (package NVR + build time per RPM) and the
actual `.rpm` payloads (file lists extracted with `rpm -qpl`). The public
`composes.fedoraproject.org` web site was not resolvable from the research
host (DNS failure); compose facts are therefore cited from the same Fedora
infrastructure that publishes the composes, `kojipkgs.fedoraproject.org/compose/`,
which is also what mkosi itself documents (see §2).

Rawhide is currently the **Fedora 46** development cycle (`.fc46` releases).
Current rawhide compose at research time: `Fedora-Rawhide-20260916.n.0`
(`https://kojipkgs.fedoraproject.org/compose/rawhide/latest-Fedora-Rawhide/COMPOSE_ID`).

## 1. Availability and versions in Rawhide

Snapshot of the rawhide repo (x86_64) on 2026-09-16, from
`https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/rpmlist.jsonl`
(67,061 RPMs; newest build in the repo: 2026-09-16 17:42 UTC — the repo is a
live, continuously moving target).

| Package | NVR in rawhide | Built (UTC) |
|---|---|---|
| systemd | `262~rc3-1.fc46` | 2026-09-15 |
| kernel | `7.3.0-0.rc3.260914g704340f1cd0d.32.fc46` | 2026-09-14 |
| dracut | `111-4.fc46` | 2026-09-09 |
| erofs-utils | `1.9.4-2.fc46` | 2026-09-10 |
| dbus-broker | `37-9.fc45` | 2026-07-15 |
| openssh | `10.5p1-1.fc46` | 2026-08-21 |
| nftables | `1.1.6-4.fc45` | 2026-07-16 |
| e2fsprogs | `1.47.4-2.fc45` | 2026-07-16 |
| kmod | `34.2-6.fc45` | 2026-07-16 |
| util-linux | `2.42.2-3.fc45` | 2026-07-22 |
| glibc | `2.44.9000-2.fc46` | 2026-09-01 |
| rust / cargo | `1.98.1-1.fc46` | 2026-09-05 |

`glibc 2.44.9000` is a development snapshot: glibc's master branch carries
`VERSION "2.42.9000"` / `RELEASE "development"` in `version.h`
(`https://raw.githubusercontent.com/bminor/glibc/master/version.h`), i.e.
`<last-release>.9000` is glibc's convention for the unreleased next version, so
rawhide's glibc is a moving pre-release of the next stable glibc, not 2.44.

All required packages are **present** in rawhide.

### 1.1 systemd 262~rc3: ukify, run0, systemd-sysupdate, systemd-repart, bootctl, systemd-bless-boot

All six tools ship in the rawhide systemd build. Verified from the actual
RPM payloads of the `systemd-262~rc3-1.fc46` build (build 3102152, per
`https://koji.fedoraproject.org/koji/buildinfo?buildID=3102152`), downloaded
from the rawhide repo:

| Tool | Subpackage that ships it (rawhide) | Path in RPM |
|---|---|---|
| `run0` | `systemd` (main) | `usr/bin/run0` |
| `ukify` | `systemd-ukify` (noarch) | `usr/bin/ukify` (+ `usr/lib/kernel/install.d/60-ukify.install`, `usr/lib/systemd/ukify/`) |
| `bootctl` | `systemd-udev` | `usr/bin/bootctl` |
| `systemd-repart` | `systemd-udev` (shared) **and** `systemd-standalone-repart` (static standalone; both `Provides: systemd-repart = 262~rc3-1.fc46`) | `usr/bin/systemd-repart` |
| `systemd-sysupdate` | `systemd-udev` | `usr/lib/systemd/systemd-sysupdate` (+ `systemd-sysupdated`, `systemd-sysupdate.service/.socket/.timer` units, `org.freedesktop.sysupdate1` D-Bus files) |
| `systemd-bless-boot` | `systemd-udev` | `usr/lib/systemd/systemd-bless-boot` (generator) + `systemd-bless-boot.service` |

**Caveat — subpackage reorganization in v262.** In this rawhide build there is
no `systemd-boot` subpackage (only `systemd-boot-unsigned`, which contains just
the EFI binaries `systemd-bootx64.efi` and the loader stubs) and no
`systemd-repart` subpackage; `ukify` was split into its own noarch subpackage,
and `bootctl`/`bless-boot`/`sysupdate` were moved into `systemd-udev`. The
classification is done by the packaging script
`split-files.py` in the rawhide spec repository
(`https://src.fedoraproject.org/rpms/systemd/raw/rawhide/f/split-files.py`),
which routes `bootctl`, `bless-boot` and `sysupdate|updatectl` into the `udev`
file list; the rawhide spec itself
(`https://src.fedoraproject.org/rpms/systemd/raw/rawhide/f/systemd.spec`)
defines the `ukify` and `standalone-repart` subpackages (with
`Provides: systemd-repart = %{version}-%{release}` on the standalone).

Consequences for the build: install by package *name/capability* (`dnf`
resolves the `Provides:`), not by subpackage path assumptions; and pin the
rawhide NVR (see §4), because the layout can change between rc releases.
Upstream man pages for all six tools exist
(`https://www.freedesktop.org/software/systemd/man/latest/ukify.html`,
`.../run0.html`, `.../bootctl.html`, `.../systemd-repart.html`,
`.../systemd-sysupdate.html`, `.../systemd-bless-boot.html`).

Note the `systemd-sysupdate` and `systemd-bless-boot` *binaries* are libexec
helpers (`/usr/lib/systemd/`); the user-facing surface is the
`systemd-sysupdate`/`systemd-sysupdated` services and the
`systemd-bless-boot.service` boot-marker service, all of which are installed.

### 1.2 kernel 7.3.0-rc3: UKI support and EROFS

- **UKI support — yes.** The `kernel-uki-virt` subpackage
  (`7.3.0-0.rc3.260914g704340f1cd0d.32.fc46`) ships a ready-made UKI:
  `/boot/efi/EFI/Linux/*-<kver>.efi`, a signed `vmlinuz-virt.efi` with
  HMAC file, and the module `config` (file list from the actual
  `kernel-uki-virt` RPM in the rawhide repo). The kernel config inside that
  RPM has `CONFIG_EFI_STUB=y` (EFI stub loader), `CONFIG_CONFIGFS_FS=y`
  (BLS/kernel configfs used by UKI boot entries), `CONFIG_EFIVAR_FS=y` and
  `CONFIG_MODULE_SIG_ALL=y` (module signing, required for signed UKIs).
  Additionally, `systemd-ukify` installs
  `usr/lib/kernel/install.d/60-ukify.install`, so `kernel-install`
  auto-generates UKIs when `ukify` is present (file list of the
  `systemd-ukify` RPM).
- **EROFS in the default kernel config — yes, as a module.** The kernel
  `config` shipped in the same RPM has `CONFIG_EROFS_FS=m` (plus
  `CONFIG_EROFS_FS_ZIP_{LZMA,DEFLATE,ZSTD}=y` and `CONFIG_EROFS_FS_ZIP_ACCEL=y`).
  The module is shipped in the `kernel-modules-core` RPM:
  `lib/modules/<kver>/kernel/fs/erofs/erofs.ko.xz` (file list of the
  `kernel-modules-core` RPM; it is *not* in `kernel-modules` or
  `kernel-modules-extra`).

### 1.3 dracut 111 and EROFS initramfs support

Upstream dracut (master, `https://github.com/dracutdevs/dracut`) contains **no
explicit EROFS handling** (zero occurrences of "erofs" in the tree). Its EROFS
support is the generic kernel-module mechanism in
`modules.d/90kernel-modules/module-setup.sh`: in non-hostonly mode dracut
installs all filesystem modules from the target kernel's module tree
("if not on hostonly mode, install all known filesystems", `instmods '=fs'`),
and in hostonly mode it installs modules for the host's root filesystem type
(`instmods "${host_fs_types[@]}"`). Since rawhide's `kernel-modules-core`
contains `kernel/fs/erofs/erofs.ko.xz`, an initramfs built in **non-hostonly**
mode (mkosi's default for initrd builds) will carry the EROFS module and can
mount an EROFS root; `erofs-utils` 1.9.4 (for `mkfs.erofs` on the build side)
is in rawhide and is already in mkosi's default Fedora tools tree (see §2).

### 1.4 The Rust set — what rawhide already packages

- `rust`, `cargo`, `rustfmt`: `1.98.1-1.fc46`; `rustup`: `1.29.0-7.fc45`.
- Prebuilt in rawhide (useful as fallback, but the plan builds from source):
  `nushell 0.99.1-8.fc46`, `uutils-coreutils 0.7.0-14.fc46`,
  `helix 25.07.1-12.fc45`.
- **Not in rawhide at all: `brush` and `zellij`** (no such packages in the
  67k-RPM rawhide repo). Brush's own README confirms it "isn't packaged in
  Fedora's official repositories"
  (`https://github.com/reubeno/brush/blob/main/README.md`). Both must be
  source-built.
- `musl-gcc 1.2.6-2.fc45` (musl C toolchain) is available; there is **no**
  `rust-std` for the `x86_64-unknown-linux-musl` target in rawhide — only
  `rust-std-static-{x86_64-unknown-uefi,x86_64-unknown-none,i686-pc-windows-gnu,...}`
  variants exist (see §3.3).

## 2. mkosi mechanics for Fedora Rawhide

From the mkosi man page (`https://github.com/systemd/mkosi/blob/main/mkosi/resources/man/mkosi.1.md`)
and the Fedora implementation (`https://github.com/systemd/mkosi/blob/main/mkosi/distribution/fedora.py`).

- **Rawhide is mkosi's default Fedora target.** `Installer.default_release()`
  returns `"rawhide"` in `fedora.py`. So `Distribution=fedora` alone targets
  rawhide; `Release=` takes the string `rawhide` (or `eln`) for Fedora.
- **Repo setup (default, no `Mirror=`):** a metalink repo
  `metalink=https://mirrors.fedoraproject.org/metalink?arch=$basearch&repo=fedora-$releasever`
  (plus disabled debuginfo/source repos). For `Release=rawhide` mkosi does
  **not** emit any updates/updates-testing repos — rawhide is rolling and has
  no updates stream (`if context.config.release != "rawhide":` guards the
  updates repos in `fedora.py`). With a `Mirror=`, the subdir becomes
  `linux/development/rawhide/$releasever/Everything` for rawhide.
- **Pinning — `Snapshot=` (frozen Koji compose).** `Snapshot=` takes a
  snapshot ID and, for Fedora, implies the Koji compose URL layout
  `compose/<release>/Fedora-<Release>-<snapshot>/compose/Everything/$basearch/os`,
  with default mirror `https://kojipkgs.fedoraproject.org` (per the man page
  "Snapshot" section and the `fedora.py` code, which comments: "Snapshot=
  pins to a frozen koji compose, which has no separate linux/updates/ tree").
  The `mkosi latest-snapshot` verb prints the current snapshot ID; its
  implementation reads
  `https://kojipkgs.fedoraproject.org/compose/rawhide/latest-Fedora-Rawhide/COMPOSE_ID`
  (`Installer.latest_snapshot()` in `fedora.py`). Current value at research
  time: `Fedora-Rawhide-20260916.n.0`, and a pinned compose repo is live at
  e.g. `https://kojipkgs.fedoraproject.org/compose/rawhide/Fedora-Rawhide-20260916.n.0/compose/Everything/x86_64/os/repodata/repomd.xml`.
  Retention on the public mirror is short: the compose directory lists only
  the most recent composes (2026-09-11 … 2026-09-16 at research time) plus a
  `latest-Fedora-Rawhide` pointer.
- **`Mirror=https://kojipkgs.fedoraproject.org` without `Snapshot=` is
  rejected** by mkosi (`die()` in `fedora.py`): "that mirror has no usable
  non-snapshot URL surface" — i.e. you cannot point at the live
  `repos/rawhide/latest` tag repo as a pseudo-pin; a Koji *tag* repo
  (`/repos/rawhide/latest/`) is regenerated as the tag moves and is not
  frozen.
- **GPG key churn is handled.** Rawhide "is a moving target and signed with a
  different GPG key every time a new Fedora release is done" (comment in
  `find_fedora_rpm_gpgkeys`, `fedora.py`); mkosi fetches the current
  rawhide key remotely from the `distribution-gpg-keys` repo and also adds the
  N-1 and N+1 release keys.
- **`metadata_expire=6h`** is set on the dnf repos when the release is
  `rawhide` or `eln` (`Installer.setup()`), to avoid mid-build metadata
  churn within a single mkosi run.
- **Tools tree is pinned the same way.** The default tools tree (a secondary
  mkosi image used to run the scripts; "useful to make image builds more
  reproducible", man page TOOLS TREES section) defaults to the host
  distribution and `ToolsTreeSnapshot=` defaults to the image's snapshot when
  distributions match — so one `Snapshot=` pins *both* the image and the
  build tooling. The default Fedora tools tree already includes
  `erofs-utils`, `e2fsprogs`, `kmod`, `dnf`, `git`, `openssl`, `sbsigntools`
  (man page table + `https://github.com/systemd/mkosi/blob/main/mkosi/resources/mkosi-tools/mkosi.conf.d/fedora/mkosi.conf`), but **not** `rust`/`cargo`.
- **Caching:** `Incremental=` caches the OS image "immediately after all OS
  packages are installed and the prepare scripts have executed"; the docs
  warn cache invalidation "is definitely not perfect", and
  `VolatilePackages=` exists exactly for "packages that change often" (rawhide
  packages) — they bypass the cache.

## 3. Building the Rust set from pinned source via cargo in mkosi's prepare phase

### 3.1 Does the stock mkosi flow support cargo builds? Yes.

The build flow (man page, "BUILD PROCESS"/script order) installs packages,
then: "Run prepare scripts on image with the `final` argument" →
"Install build packages in overlay if any build scripts are configured" →
"Run prepare scripts on overlay with the `build` argument" → "Run build
scripts on image + overlay" → "Copy the build scripts outputs into the image".
The documented levers:

- **`PrepareScripts=`** — "has network access and may be used to install
  packages from other sources than the distro's package manager (e.g. pip,
  npm, …)"; run with the `final` argument right after package installation,
  then a second time with the `build` argument on the build overlay.
- **`BuildPackages=`** — "configures packages to install only in an overlay
  that is made available on top of the image to the prepare scripts when
  executed with the `build` argument and the build scripts … packages listed
  here will be absent in the final image." This is the stock mechanism for
  a throwaway toolchain: `BuildPackages=rust,cargo` puts rustc/cargo in the
  build overlay only.
- **`BuildSources=`** — mounts host source directories (e.g. pinned
  checkouts of the five Rust projects) into `$SRCDIR` for the scripts.
- **`BuildScripts=` (`mkosi.build`)** — runs with `$DESTDIR`; "the contents of
  `$DESTDIR` are copied into the image". A `cargo build --release` in
  `mkosi.prepare` (build arg) or `mkosi.build` that places the five binaries
  in `$DESTDIR/usr/bin/` is the stock pattern; `PackageDirectories=`/`$PACKAGEDIR`
  additionally allow publishing the built artifacts as local RPMs.

So: `BuildPackages=rust,cargo` (or `ToolsTreePackages=rust,cargo` to add them
to the pinned default tools tree) + `BuildSources=` for the pinned sources +
a prepare/build script running `cargo build --release --locked` is fully
within stock mkosi; no plugin or fork is needed.

### 3.2 Toolchain requirements

- Rawhide provides `rust`/`cargo` **1.98.1** (2026-09-05 build) for x86_64;
  `rustup 1.29.0` is available if a different toolchain/target is needed.
- Target features: native `x86_64-unknown-linux-gnu` is the default and
  matches the image architecture; edition 2024 crates require rust ≥ 1.85 —
  comfortably covered by 1.98.1.
- MSRV check for the headline crate: brush's `brush-shell` crate declares
  `rust-version = "1.95.0"` (workspace `rust-version = "1.88.0"`) in
  `https://github.com/reubeno/brush/blob/main/brush-shell/Cargo.toml` —
  satisfied by rawhide's 1.98.1. (The other four — nushell, helix,
  uutils-coreutils, zellij — have similar MSRVs well below 1.98; each pinned
  checkout's `Cargo.toml` is the authority at build time.)
- Reproducibility of the cargo step itself: `cargo build --locked` fails if
  `Cargo.lock` would change and `--offline`/`--frozen` prevent any registry
  access (Cargo book, `https://doc.rust-lang.org/cargo/commands/cargo-build.html`;
  lock-file semantics in
  `https://doc.rust-lang.org/cargo/reference/resolver.html` §"Lock file" and
  `https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html`).
  Pin each crate by git tag/commit (e.g. brush `brush-v0.4.0`,
  `https://github.com/reubeno/brush`) and keep `Cargo.lock` in the build
  source; the Rust set then has no dependency on rawhide moving.

### 3.3 Brush as `/bin/sh`

From the upstream project (`https://github.com/reubeno/brush`, latest tag
`brush-v0.4.0`; README + `brush-shell/Cargo.toml`):

- **Upstream positioning/guidance:** brush is "a modern bash- and
  POSIX-compatible shell written in Rust", "ready for use as a daily
  driver", "validated against bash with ~1700 compatibility tests", with a
  published Compatibility Reference for gaps ("Not everything works yet:
  `select` and some edge cases aren't supported", README). Upstream does
  **not** explicitly document or recommend installation as `/bin/sh`; the
  compatibility position (POSIX-compatible, bash-daily-driver, tested
  against bash) is the basis on which `/bin/sh` duty is argued. `select` is
  a bashism, not a POSIX `sh` requirement, but the compatibility reference
  should be re-checked against the POSIX `sh` surface before cut-over.
- **Static linking / bundled libc:** the upstream crate has **no static-link
  feature** — the `[features]` of `brush-shell` (v0.4.0 and main) are
  `default/basic/minimal/reedline/experimental*/schema`, nothing musl- or
  static-related. A fully static binary therefore requires building for
  `x86_64-unknown-linux-musl` via `rustup target add` (rustup 1.29 is in
  rawhide); rawhide does **not** package a Rust musl std for that target
  (only `rust-std-static-*` for `none`/`uefi`/windows/wasm triples).
  The `musl-gcc 1.2.6` package covers the C side only.
- **Dynamic linking is a viable alternative:** linked against the image's own
  glibc (rawhide's `2.44.9000`), a dynamically linked brush in
  `/usr/bin/brush` + `/bin/sh` symlink is the same shape as dash/bash on any
  Linux and needs no libc bundling. For an immutable image this is the lower-risk
  path; static/musl is the belt-and-braces path and needs the extra rustup
  target step.
- **Not a distro package:** brush is absent from rawhide (verified in §1.4;
  README: "isn't packaged in Fedora's official repositories", community
  Terra repo aside) — source build in the prepare phase (or `PackageDirectories=`
  + local repo) is required either way. Zellij is likewise absent.

## 4. Churn and reproducibility of a moving Rawhide target

**What moves.** Rawhide is a rolling tag: the rawhide repo's newest build at
research time was same-day (2026-09-16 17:42 UTC); systemd/kernel were rebuilt
in the preceding 2 days, glibc 2 weeks earlier. There is no updates stream for
rawhide (mkosi emits no updates repo for it, §2), so every push to the
`rawhide` tag changes what a `Release=rawhide` build sees. The Koji *tag* repo
(`repos/rawhide/latest`) is regenerated as the tag moves and is explicitly not
usable as a pin (mkosi rejects it, §2).

**Pinning mechanisms that exist:**

1. **`Snapshot=` → frozen Koji compose** (primary mechanism). A compose is a
   frozen snapshot of the rawhide tag at compose time; the rawhide compose is
   produced daily (`Fedora-Rawhide-YYYYMMDD.n.0`; current:
   `Fedora-Rawhide-20260916.n.0`,
   `https://kojipkgs.fedoraproject.org/compose/rawhide/COMPOSE_ID` via the
   `latest-Fedora-Rawhide` symlink). mkosi turns the ID into
   `compose/rawhide/Fedora-Rawhide-<ID>/compose/Everything/...` URLs for both
   the image and (by default) the tools tree.
2. **`mkosi latest-snapshot`** verb to discover/bump the pinned ID
   (man page: "useful to automatically bump snapshots every so often").
3. **`LocalMirror=` / `Mirror=`** against a frozen or self-hosted copy of a
   compose (any "kojipkgs-compatible" mirror works per `fedora.py`), for
   offline or long-retention pinning.
4. **`VolatilePackages=`** to keep fast-moving rawhide packages out of the
   `Incremental=` cache.
5. **`Cargo.lock` + `cargo build --locked/--frozen`** for the Rust set, which
   is independent of rawhide entirely once the toolchain is pinned.

**What breaks when rawhide moves under an unpinned build:**

- NVR drift between two builds a day apart: different kernel NVR (new UKI /
  initramfs inputs), systemd NVR, glibc NVR — output images differ with no
  source change.
- **GPG key rotation at each Fedora release:** rawhide is re-signed with the
  new release's key; mkosi compensates by fetching the current key plus N-1/N+1
  (`fedora.py`), but a build pinned to a compose from the *previous* release
  cycle still needs the old key available.
- **Package/subpackage reshuffles:** the systemd 262 rc cycle already moved
  `ukify` into its own subpackage, `bootctl`/`bless-boot`/`sysupdate` into
  `systemd-udev`, and replaced `systemd-boot`/`systemd-repart` subpackages with
  `systemd-boot-unsigned`/`systemd-standalone-repart` (§1.1). Hardcoded
  subpackage names in `Package=` lists can stop resolving or pick up different
  files between rawhide states; capability-based names (backed by `Provides:`)
  and NVR pinning are the mitigations.
- **Removals/renames of any build-input package** (e.g. a package dropped from
  rawhide) turn a previously green build into a hard dnf failure with no
  metadata to fall back to.
- **Cache staleness:** `Incremental=` invalidation is "rudimentary" (man
  page); combined with a moving repo, a stale cached image is a reproducibility
  hazard — pin the snapshot and treat the cache as derived from it.

**What breaks with a *pinned* compose over time:**

- **Retention on the public mirror is short** — at research time only ~7
  recent composes were listed under `kojipkgs.fedoraproject.org/compose/rawhide/`
  (2026-09-11 … 2026-09-16). Once a pinned compose ID is pruned from the
  mirror, the `Snapshot=` URLs 404 and the build is no longer reproducible
  anywhere. For durable reproducibility the chosen compose must be archived
  (self-hosted mirror via `Mirror=`/`LocalMirror=`, or mirror the compose
  directory) before it is pruned.
- Everything *inside* the compose is frozen (that is the point): the RPM set,
  the NVRs, and the repo metadata (repodata checksums) are fixed for the life
  of the files.

## 5. Bottom line

**Feasible, no show-stopper.**

1. All required packages are in rawhide (fc46): systemd 262~rc3 ships all six
   tools (ukify/run0/sysupdate/repart/bootctl/bless-boot) — albeit re-homed into
   new subpackages in v262; kernel 7.3-rc3 ships UKI (prebuilt
   `kernel-uki-virt`, `CONFIG_EFI_STUB=y`) and EROFS
   (`CONFIG_EROFS_FS=m`, `erofs.ko.xz` in `kernel-modules-core`);
   dracut 111 / erofs-utils 1.9.4 / dbus-broker 37 / openssh 10.5 / nftables
   1.1.6 / e2fsprogs 1.47.4 / kmod 34.2 / util-linux 2.42.2 / glibc 2.44.9000.
2. mkosi targets rawhide natively (`Release=rawhide` is the default) and has a
   first-class pin: `Snapshot=` on a frozen Koji compose, with
   `latest-snapshot` to discover IDs and the same snapshot defaulting into the
   tools tree.
3. Cargo builds from pinned source are a stock flow
   (`BuildPackages=rust,cargo` + `BuildSources=` + prepare/build scripts +
   `$DESTDIR`); rawhide's rust 1.98.1 covers brush's 1.95 MSRV.
4. Brush as `/bin/sh` is workable (POSIX/bash-compatible, ~1700 compat tests,
   documented gaps to review) but needs a source build (not in Fedora) and,
   only for the static variant, a rustup musl target rawhide does not package.
5. The main operational risks are churn, not availability: pin the compose
   (`Snapshot=`) for reproducibility, pin cargo via `Cargo.lock`/`--locked`,
   install systemd components by capability (not subpackage path), and
   **archive the pinned compose off the public mirror** before it is pruned
   (only ~1 week of rawhide composes is retained there), or use a
   self-hosted frozen mirror via `LocalMirror=`.

### Source index

- Rawhide repo NVRs/buildtimes: `https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/rpmlist.jsonl`
- RPM payloads (file lists): `https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/toplink/packages/<pkg>/<ver>/<rel>/<arch>/<rpm>`
  (systemd 262~rc3-1.fc46; kernel 7.3.0-0.rc3.260914g704340f1cd0d.32.fc46)
- Koji build page: `https://koji.fedoraproject.org/koji/buildinfo?buildID=3102152`
- Compose index/COMPOSE_ID: `https://kojipkgs.fedoraproject.org/compose/rawhide/`,
  `https://kojipkgs.fedoraproject.org/compose/rawhide/latest-Fedora-Rawhide/COMPOSE_ID`
- systemd rawhide spec + split-files.py: `https://src.fedoraproject.org/rpms/systemd/raw/rawhide/f/systemd.spec`,
  `https://src.fedoraproject.org/rpms/systemd/raw/rawhide/f/split-files.py`
- systemd man pages: `https://www.freedesktop.org/software/systemd/man/latest/{ukify,run0,bootctl,systemd-repart,systemd-sysupdate,systemd-bless-boot}.html`
- glibc versioning convention: `https://raw.githubusercontent.com/bminor/glibc/master/version.h`
- mkosi man page: `https://github.com/systemd/mkosi/blob/main/mkosi/resources/man/mkosi.1.md`
- mkosi Fedora implementation: `https://github.com/systemd/mkosi/blob/main/mkosi/distribution/fedora.py`
- mkosi default tools tree: `https://github.com/systemd/mkosi/blob/main/mkosi/resources/mkosi-tools/mkosi.conf(.d/fedora/mkosi.conf)`
- dracut source (fs module logic): `https://github.com/dracutdevs/dracut/blob/master/modules.d/90kernel-modules/module-setup.sh`
- brush: `https://github.com/reubeno/brush` (README, `brush-shell/Cargo.toml`, tag `brush-v0.4.0`)
- Cargo book: `https://doc.rust-lang.org/cargo/commands/cargo-build.html`,
  `https://doc.rust-lang.org/cargo/reference/resolver.html`,
  `https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html`
