# Ingot operator entrypoint.
#
#   just build                 build the pinned release (0.1.0, slot A) from
#                              the archived compose
#   just build <flags>         extra flags pass through to tools/build.sh
#                              verbatim (e.g. --version 0.2.0 --slot b)
#   just harness               T1 boot harness (probe injected into the
#                              working disk's /var; deep invariants)
#   just prod                  T1 gate, probe-free: host-side evidence only
#   just ab                    T2 A/B selection + automatic rollback scenario
#   just test                  host-side unittest suite (fast, no VM)

# Build the pinned release against the archived compose; extra args pass
# through to tools/build.sh verbatim. The mkosi sandbox needs root on
# this workstation (unprivileged user namespaces are blocked), so the
# build runs via passwordless sudo.
build *args:
	sudo tools/build.sh --local {{args}}

# T1 boot harness: boots the deployed disk and asserts the boot invariants
# (probe mode: the probe is injected into the working disk's /var).
harness:
	harness/run.sh

# T1 gate in prod mode: no probe, host-side evidence only (serial console,
# kernel cmdline, ESP forensics).
prod:
	python3 harness/run.py --no-probe

# T2 scenario: A/B selection, bless, and automatic rollback (5 boots).
ab:
	python3 harness/run_ab.py

# T4 installer: the declarative install engine. The TOML config is
# the authorization (spec 11.4); run with --dry-run first to
# inspect the plan. Root for disk operations; file targets need a
# work volume with at least the disk's virtual size free (use
# --work; /run is tmpfs and may be smaller than the disk).
install *args:
	sudo src/target/release/ingot-installer {{args}}

# Host-side unittest suite (no VM, seconds).
test:
	python3 -m unittest discover -s harness -p 'test_*.py' -v

# Rust workspace tests (offline; the helper's fixtures are committed in
# src/update-helper/tests/).
rust-test:
	cd src && cargo test
