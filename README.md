# Ingot

Ingot is a minimal, immutable, systemd-first Linux operating system for x86_64 UEFI systems, designed for headless servers and terminal-driven technical workstations.

It combines an immutable `/usr` payload model with dual-slot A/B atomic updates, Unified Kernel Images (UKI) signed for Secure Boot, and a modern Rust-first terminal userland.

## Architecture

- **Immutable Payload Slots:** Operating system versions live in read-only `erofs` partitions (`usr-x86-64`). The active slot is mounted read-only at `/usr`.
- **Ephemeral Runtime Root:** The root filesystem (`/`) is a transient `tmpfs` created during early boot. Persistent host configuration (`/etc`) is stored in `/var/lib/etc` and bind-mounted over `/etc`, initialized on first boot from `/usr/share/factory/etc`.
- **A/B Updates & Automatic Rollback:** Two OS slots (Slot A and Slot B). New versions deploy to the inactive slot with a tries-left boot counter. If an update fails to reach `multi-user.target`, `systemd-boot` automatically falls back to the previous known-good slot.
- **Unified Kernel Images (UKI):** Kernel, initramfs, CPU microcode, and kernel command line are bundled and signed into a single UEFI binary executed directly by `systemd-boot`.
- **Three-Tier Shell Model:** `bash` in the initramfs (dracut interpreter), `brush` as the system POSIX shell (`/bin/sh`), and `nushell` as the default interactive administrative shell.
- **Modern Terminal Userland:** Ships with Nushell, Helix editor (`hx`), Zellij terminal multiplexer, and uutils Coreutils.
- **Installation:** Includes both a declarative unattended installer and an interactive ncurses-style TUI wizard (`ingot-installer --wizard`) that deploys directly from the live ISO media.

## Documentation

- [Formal Design Specification](docs/SPECS.md): Canonical reference for system architecture and acceptance criteria.
- [Domain Glossary (CONTEXT.md)](CONTEXT.md): Project vocabulary and locked architectural decisions.
- [Architectural Decision Map](docs/MAP.md): Wayfinder resolution history and lineage.
- [Agent Guidelines](AGENTS.md): Coding guidelines and workflows.

## Quickstart

### Prerequisites

- Linux workstation with KVM virtualization enabled.
- Host dependencies: `mkosi` (v26+), `qemu-system-x86_64` (v10.2+), `ovmf` (with snakeoil keys enrolled), `xorriso`, `mtools`, `dosfstools`, `sbsigntool`.
- Passwordless `sudo` configured for build commands (required for the `mkosi` container sandbox).
- Python 3.12+ (managed with `uv`).

Initialize git submodules for upstream userland components:

```sh
git submodule update --init --depth 1
```

### Build Workflow

The repository uses `just` as the command runner:

```sh
# Build the default release image (Ingot 0.1.0 in slot A)
just build

# Build a synthetic update image into slot B (for A/B update testing)
just build --version 0.2.0 --slot b

# Assemble the live hybrid bootable ISO
tools/build-iso.sh
```

### Testing & Verification

```sh
# Run fast host-side unit tests (Python pytest + Rust cargo test)
just test
just rust-test

# Test the live ISO interactively in QEMU (Secure Boot ON, drops into Nushell)
just test-iso

# Run the T1 boot invariant gate (asserts UKI, erofs, tmpfs root, Secure Boot)
just harness

# Run the 5-boot A/B update, blessing, and automatic rollback scenario
just ab

# Run the end-to-end unattended install and live ISO verification harness
just iso
```

## Repository Structure

```text
├── docs/           # Formal specs, domain decisions, and architecture maps
├── image/          # mkosi image build definitions, dracut modules, factory defaults
├── src/            # Rust workspace (ingot-installer and ingot-update-helper)
├── harness/        # Automated QEMU/OVMF test harnesses and verification suites
├── tools/          # Build scripts, compose mirroring, pins, and signing keys
├── thirdparty/     # Pinned git submodules for upstream Rust userland
└── dist/           # Built disk images, UKIs, and ISO artifacts (git-ignored)
```

## Security Notice

The UKIs and systemd-boot binaries in this repository are signed with the EDK2 snakeoil test key located in `tools/keys/`. The test harness runs OVMF firmware enrolled with these snakeoil keys. These keys are strictly for local testing and CI verification; never use them in production deployments.
