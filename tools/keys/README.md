# Secure Boot snakeoil test keys

Snakeoil test keys from the edk2 project, shipped by Ubuntu in the
`ovmf` package (`/usr/share/ovmf/PkKek-1-snakeoil.{key,pem}`) and by
Fedora in `edk2-ovmf-snakeoil`.

- `snakeoil.pem` - the edk2 "snakeoil" PK/KEK/DB signing certificate
  (identical upstream across distros).
- `snakeoil.key` - the matching RSA private key, decrypted
  (`openssl rsa -passin pass:snakeoil`). The distro ships it PKCS#8
  encrypted with the well-known snakeoil passphrase; the decrypted key
  is committed because the build must run reproducible and
  self-contained without interactive passphrase entry.

This is a **test key, not a real key**: anything signed with it is
untrusted and verifiable only against the matching snakeoil CA chain.
It is used exclusively for the QEMU Secure Boot harness (pinned OVMF
vars image with these keys enrolled as PK/KEK/DB) and for signing the
UKI and the systemd-boot fallback. It is never used for production
firmware.

Provenance (distro package files, Ubuntu 26.04 "Ondat" `ovmf` 2025.11-3ubuntu7):

    $ sha256sum /usr/share/ovmf/PkKek-1-snakeoil.{key,pem}
    3473e7581c66d6c0f1d9cde1c454b81f8d1bc5991621913e2edfca2af3f315f3  /usr/share/ovmf/PkKek-1-snakeoil.key
    d1ead44fc7748b8c26e0cba2141b0b54a379c0f11c4e593fbc5d7dd995f90265  /usr/share/ovmf/PkKek-1-snakeoil.pem

(Commit the sha256 of the *encrypted* distro key above; the committed
`snakeoil.key` decrypts from it.)

The harness pins (a) the OVMF firmware blobs it boots with and (b)
these key files by sha256 in `harness/pins.json`; a mismatch fails the
harness before boot.
