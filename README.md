# Ingot

A minimal, immutable, systemd-first, x86_64 UEFI Linux OS for headless servers and terminal-driven technical workstations: A/B OS payload slots (erofs, /usr-only), UKI boot via systemd-boot with Automatic Boot Assessment, atomic updates via systemd-sysupdate, and a Rust userland (brush, nushell, helix, zellij, uutils) built from pinned upstream sources.

**Status:** design/decision phase (wayfinder map). The build follows the to-spec handoff.

- Canonical design reference: [docs/SPECS.md](docs/SPECS.md)
- Decision map: [classy-giraffe/Ingot#1](https://github.com/classy-giraffe/Ingot/issues/1) (wayfinder)
- Domain glossary: [CONTEXT.md](CONTEXT.md)
- Research findings: [docs/research/](docs/research/)
- Agent conventions: [docs/agents/](docs/agents/)
