# Architecture and Domain Rules

Architectural invariants and domain decisions for Ingot.

## Slot Payload Model

- A slot is strictly the `/usr` payload tree (GPT partition type `usr-x86-64`).
- Kernel and initramfs live in per-slot Unified Kernel Images (UKI) on the ESP, never in the slot partition.
- Slot identity uses versioned GPT labels: `ingot_<v>` for populated slots, `_empty` for free slots.
- Dual-slot A/B updates: instances max is 2. Updates write whole-image EROFS (`ingot_<v>.root.erofs`) to the inactive slot while the active slot serves running workloads.

## Runtime Root and State Persistence

- Root filesystem (`/`) is an ephemeral `tmpfs` created by initramfs on early boot.
- `/var` and `/home` live on dedicated btrfs partitions, persistent across updates and reboots.
- Host configuration in `/etc` is a bind mount over `/var/lib/etc`. Factory defaults ship in the slot at `/usr/share/factory/etc`.
- Idempotent oneshot copies factory files on first boot and performs new-files-only merge on update (factory files never overwrite host modifications).

## Pinned Rust Userland

- All userland terminal utilities are compiled from pinned upstream source in `thirdparty/` git submodules:
  - `brush` (system POSIX shell, `/bin/sh`, dynamic glibc)
  - `nushell` (interactive default shell, `/usr/bin/nushell`)
  - `helix` (modal text editor, `/usr/bin/hx`)
  - `zellij` (terminal workspace multiplexer, `/usr/bin/zellij`)
  - `uutils coreutils` (shadows distro coreutils in slot payload)
- Workspace crates `ingot-installer` and `ingot-update-helper` live under `src/`.
- Never substitute distro packages for the pinned Rust layer. Pins are enforced by `tools/check-pins.py` against `tools/pins.json` and `.gitmodules`.

## Boot and Security

- Boot is UEFI-only via `systemd-boot` and per-slot UKIs embedding kernel, initramfs, command line, CPU microcode, and OS release metadata.
- Secure Boot enforced from first boot using the EDK2 snakeoil test key in v1 (`tools/keys/snakeoil.key`).
- Automatic boot assessment and rollback via boot-attempt counters and `systemd-bless-boot`.
