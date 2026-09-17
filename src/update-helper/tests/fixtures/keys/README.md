# Fixture GPG keys

Throwaway keys for the ingot-update-helper tests. NEVER use these for
real releases: the real project key lives outside this repo (release
pipeline) and is the only key the slot's vendor keyring will carry in
production.

- project.pgp / project.pub / project.sec: the "project" key
  (Ingot Fixture Project <project@ingot.local>, ed25519, no expiry).
  project.pgp is the binary keyring the tests pass as --keyring
  (the same shape as /usr/lib/systemd/import-pubring.pgp); project.pub
  is the armored form.
- other.pgp / other.pub / other.sec: a second key
  (Ingot Fixture Other <other@ingot.local>) used to sign the
  unknown-key fixture; it is NOT in the project keyring.

Fingerprints (recorded so the fixtures are auditable):

- project: D6D121550BD1589D44798F1B5A777E4D9EA4CF2A
- other:   6EE31306B576F82A7F1CF5D626016492BBD0C507
