# Release Pipeline and Publishing Rules

Invariants and procedures for packaging, signing, and publishing Ingot releases.

## Asset Contract

A release produces a fixed set of ten assets in `dist/`:
- `ingot_<v>.root.erofs` and `ingot_<v>.root.erofs.gpg` (slot payload)
- `ingot_<v>.efi` and `ingot_<v>.efi.gpg` (signed UKI)
- `ingot_<v>.iso` and `ingot_<v>.iso.gpg` (live hybrid ISO)
- `SHA256SUMS` and `SHA256SUMS.gpg` (strict GNU sha256sum binary format)
- `manifest.json` and `manifest.json.gpg` (v1 schema release descriptor)

The manifest is deliberately omitted from `SHA256SUMS`. Every asset listed in `SHA256SUMS` must appear in `manifest.json`'s `assets` array with matching SHA256.

## Size Invariant

The compressed root filesystem (`ingot_<v>.root.erofs`) has a hard 2 GiB ceiling:
- Any release whose compressed image exceeds 2,147,483,648 bytes (2 GiB) must fail the build with a clear error.
- The root payload is extracted from `ingot_<v>.slot.raw` to its actual EROFS superblock filesystem size (`blocks * block_size`), stripping the partition zero-padding.

## Key Management and Trust

- Long-lived project GPG release key signs `manifest.json`, `SHA256SUMS`, and detached signatures for payload assets.
- Public key must ship in the slot vendor keyring at `image/files/usr/lib/systemd/import-pubring.pgp` and repo at `tools/keys/project.pgp` and `tools/keys/project.pub`.
- Private key (`tools/keys/project.sec`) must remain local on the workstation (mode 0600) and must stay gitignored. Never commit private keys.

## Publish Gate

- Gated strictly on a green harness result (`dist/harness/results.json` with `pass: true` and non-empty passing checks).
- Releases on GitHub Releases are immutable: no yank, no replace. The publish tool refuses to overwrite an existing tag.
- Pre-upload validation re-verifies all detached GPG signatures against `tools/keys/project.pgp`.

## Verification Seam

- Every release must verify end-to-end through `ingot-update-helper --repo classy-giraffe/Ingot --keyring tools/keys/project.pgp`.
- Helper authenticates `manifest.json` and `SHA256SUMS` via GPG, cross-checks hashes, and emits the pin document.
