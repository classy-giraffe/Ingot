# Context

Glossary of Ingot's domain vocabulary. Use these terms in tickets, map entries, and spec text; do not drift to synonyms. Terms enter here when a wayfinder decision session resolves them; the resolution tickets are the authority for each definition.

## Terms

### Slot

An immutable `/usr` payload partition (GPT type `usr-x86-64`) holding exactly one OS version: the `/usr` tree with all registry components, `/usr/lib/modules` for the release kernel, factory defaults, and the shipped sysupdate transfers. The kernel binary and initramfs are NOT in the slot (they are in the UKI on the ESP). A/B means "the two newest versions" (`InstancesMax=2`); slot identity is the versioned GPT label `ingot_<v>`, with `_empty` marking a free slot. Updates replace a slot's whole partition image; rollback boots a previous slot. Extensions (sysexts) layer on top of a slot and persist across slot switches.

Resolved by [Lock the slot payload model (/usr-only slots, minimal runtime root)](https://github.com/classy-giraffe/Ingot/issues/3).

### Runtime root

The minimal `/` generated at boot: a tmpfs root carrying the usr symlinks (`/bin` `/sbin` `/lib` `/lib64` -> `usr/...`), empty top-level directories (`/opt` `/srv` `/media` `/mnt` `/root`), and ephemeral mounts (`/dev` `/proc` `/sys` `/run` `/tmp`), plus `/efi` read-write. The initramfs mounts `/var` and binds `/var/lib/etc` over `/etc`, then mounts the active slot read-only at `/usr` (identified by the PARTUUID in the UKI's kernel command line), before switch-root; systemd mounts `/home`, `/efi`, `/tmp` at `local-fs.target`. No custom mount units.

Resolved by [Lock the slot payload model (/usr-only slots, minimal runtime root)](https://github.com/classy-giraffe/Ingot/issues/3).

### Shell

The three-tier shell model: `bash` is the initramfs shell (the dracut shell-interpreter); `brush` is the runtime root's `/bin/sh` (the POSIX shell, a pinned Rust build); `nushell` is the interactive admin shell (spec 3.1). The nushell package ships `/usr/bin/nu`; the image provides `/usr/bin/nushell` as a symlink, which is the path the installer's default user shell references.

Resolved by project-owner decision (2026-09-18, issue #18 thread): bash for the initramfs, brush as the system POSIX shell, nushell as the interactive shell.
