# Ingot

A minimal, immutable, systemd-first, x86_64 UEFI Linux OS for headless
servers and terminal-driven technical workstations: A/B OS payload slots
(erofs, /usr-only), UKI boot via systemd-boot, atomic updates via
systemd-sysupdate, and a Rust userland (brush, nushell, helix, zellij,
uutils) built from pinned upstream sources.

**Status:** T1 + T2 done - first bootable slot (issue #15) and A/B
selection with automatic rollback (issue #16). See RESUME-T1.md and
RESUME-T2.md for details.

- Canonical design reference: [docs/SPECS.md](docs/SPECS.md)
- Decision map: [classy-giraffe/Ingot#1](https://github.com/classy-giraffe/Ingot/issues/1) (wayfinder) - repo snapshot: [docs/MAP.md](docs/MAP.md)
- Domain glossary: [CONTEXT.md](CONTEXT.md)
- Research findings: [docs/research/](docs/research/)
- Agent conventions: [docs/agents/](docs/agents/)

## Building

The build chain: pinned Fedora Rawhide compose -> mkosi (single main
image, disk output) -> GPT disk with signed UKI + fallback on the ESP,
the slot erofs (zstd), and btrfs state partitions -> QEMU/OVMF Secure
Boot harness. The erofs slot and the UKI are also emitted as standalone
split artifacts.

The Rust userland (brush, nushell, helix, zellij, uutils coreutils)
is pinned upstream source in the repo: git submodules under
`thirdparty/`, one per component, checked out at the upstream commit
recorded in `tools/pins.json` (the pin check enforces it). After
cloning: `git submodule update --init --depth 1`.

```sh
# 1. (optional) archive the pinned compose for offline rebuilds:
tools/archive-compose.sh

# 2. build (pin check, mkosi, erofs, deploy). --version/--slot build a
# different release into a different A/B slot (T2); per-version
# artifacts: dist/ingot_<v>.raw, dist/ingot_<v>.slot.raw,
# dist/ingot_<v>.efi:
just build                       # pinned release (0.1.0, slot A)
just build --version 0.2.0 --slot b

# 3. boot + assert (exit code + machine-readable results):
just harness                     # T1 gate, probe mode
just prod                        # T1 gate, probe-free
just ab                          # T2: A/B selection, bless, rollback
just test                        # host-side unit tests (no VM)
```

`dist/build-metadata-<v>.json` records the per-version build inputs
(compose ID, kernel version, the Rust userland pins) and the erofs
parameters (spec 20.4).

- `tools/` - build orchestration, compose mirroring, pins, snakeoil keys
- `image/` - mkosi project: the single main image (whole-system disk
  output) with its phase scripts, the 99ingot dracut module and factory
  defaults (`files/`), and the fixed-UUID repart baselines
- `thirdparty/` - git submodules: the pinned upstream Rust userland
  sources (one submodule per component, at the pinned commit)
- `harness/` - QEMU/OVMF boot harness with machine-readable assertions:
  the T1 boot gate (probe and prod modes) and the T2 A/B scenario
- `dist/` - build artifacts (git-ignored)

### Requirements

Host tools are version-pinned in `harness/pins.json` and checked by
`tools/build.sh` before any build: mkosi 26, qemu 10.2.1 with KVM,
OVMF snakeoil firmware. The build runs as root via passwordless sudo
(configured for the build user on this workstation): the mkosi sandbox
needs root here (unprivileged user namespaces are blocked). The
harness and its unit tests run unprivileged.

### Security Boot

The UKI and the systemd-boot fallback are signed with the edk2 snakeoil
test key ([tools/keys/](tools/keys/)); the harness boots pinned OVMF
with the matching snakeoil keys enrolled as PK/KEK/DB. This is a test
key - never use it outside the harness.
