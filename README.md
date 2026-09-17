# Ingot

A minimal, immutable, systemd-first, x86_64 UEFI Linux OS for headless
servers and terminal-driven technical workstations: A/B OS payload slots
(erofs, /usr-only), UKI boot via systemd-boot, atomic updates via
systemd-sysupdate, and a Rust userland (brush, nushell, helix, zellij,
uutils) built from pinned upstream sources.

**Status:** T1 done - first bootable slot (issue #15): build chain,
harness 10/10, reproducible slot erofs. See RESUME-T1.md for details.

- Canonical design reference: [docs/SPECS.md](docs/SPECS.md)
- Decision map: [classy-giraffe/Ingot#1](https://github.com/classy-giraffe/Ingot/issues/1) (wayfinder) - repo snapshot: [docs/MAP.md](docs/MAP.md)
- Domain glossary: [CONTEXT.md](CONTEXT.md)
- Research findings: [docs/research/](docs/research/)
- Agent conventions: [docs/agents/](docs/agents/)

## Building (T1)

The build chain: pinned Fedora Rawhide compose -> mkosi (single main
image, disk output) -> GPT disk with signed UKI + fallback on the ESP,
the slot erofs (zstd), and btrfs state partitions -> QEMU/OVMF Secure
Boot harness. The erofs slot and the UKI are also emitted as standalone
split artifacts.

```sh
# 1. (optional) archive the pinned compose for offline rebuilds:
tools/archive-compose.sh

# 2. build everything (pin check, mkosi, erofs, deploy):
tools/build.sh                # network build against the pinned compose
tools/build.sh --local        # offline build from the archived compose

# 3. boot + assert (exit code + dist/harness/results.json):
harness/run.sh
```

`dist/build-metadata.json` records the build inputs (compose ID, kernel
version, brush SHA) and the erofs parameters (spec 20.4).

- `tools/` - build orchestration, compose mirroring, pins, snakeoil keys
- `image/` - mkosi project: the single main image (whole-system disk
  output) with its phase scripts, the 99ingot dracut module and factory
  defaults (`files/`), and the fixed-UUID repart definitions
  (`mkosi.repart/`)
- `harness/` - QEMU/OVMF boot harness with machine-readable assertions
- `dist/` - build artifacts (git-ignored)

### Requirements

Host tools are version-pinned in `harness/pins.json` and checked by
`tools/build.sh` before any build: mkosi 26, qemu 10.2.1 with KVM,
OVMF snakeoil firmware. The build runs as root via the passwordless
`tools/ingot-build` sudo wrapper (`/etc/sudoers.d/ingot`): the mkosi
sandbox needs root on this workstation (unprivileged user namespaces
are blocked).

### Security Boot

The UKI and the systemd-boot fallback are signed with the edk2 snakeoil
test key ([tools/keys/](tools/keys/)); the harness boots pinned OVMF
with the matching snakeoil keys enrolled as PK/KEK/DB. This is a test
key - never use it outside the harness.
