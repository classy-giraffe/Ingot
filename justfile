# Ingot operator entrypoint.
#
#   just build                 build the pinned release (0.1.0, slot A) from
#                              the archived compose
#   just build <flags>         extra flags pass through to tools/build.sh
#   just release               build the signed release asset set (T7)
#   just publish               publish the release gated on harness (T7)
#   just harness               T1 boot harness (probe injected into the
#                              working disk's /var; deep invariants)
#   just prod                  T1 gate, probe-free: host-side evidence only
#   just test                  host-side unittest suite (fast, no VM)
#   just iso                   T6 live ISO E2E (issue #23, criteria
#                              11/12): live boot + unattended install

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

# Host-side unittest suite (fast, no VM).
test *args:
	@if [ -x .venv/bin/pytest ]; then \
		.venv/bin/pytest harness/test_*.py {{args}}; \
	else \
		python3 -m unittest discover -s harness -p 'test_*.py' -v {{args}}; \
	fi

# Python linting and formatting via ruff.
lint *args:
	@if [ -x .venv/bin/ruff ]; then \
		.venv/bin/ruff check harness/ tools/ {{args}}; \
	else \
		echo "ruff not found; install with: uv pip install ruff"; \
	fi

fmt *args:
	@if [ -x .venv/bin/ruff ]; then \
		.venv/bin/ruff format harness/ tools/ {{args}}; \
	else \
		echo "ruff not found; install with: uv pip install ruff"; \
	fi
# Rust workspace tests (offline; the helper's fixtures are committed in
# src/update-helper/tests/).
rust-test:
	cd src && cargo test

# T6 live ISO E2E (issue #23, criteria 11/12): boots the ISO
# headless (Secure Boot on) and asserts SSH + ephemerality, then runs
# the unattended install against a declarative config disk and boots
# the installed machine. Root (KVM, the config-disk loop mount).
iso:
	sudo python3 harness/iso.py

# Manually test the live ISO interactively: boots QEMU with KVM, 4 cores,
# 8 GB RAM, Secure Boot on (OVMF snakeoil keys enrolled), and an attached
# 32 GB virtual HDD (dist/test-target.raw). Connects the serial console
# to stdio (exit with 'poweroff' in guest, or 'Ctrl-A x' in QEMU). Also
# forwards SSH to host port 2222 (ssh -i tools/keys/live.key -p 2222 root@127.0.0.1).
test-iso *extra:
	@mkdir -p dist
	@[ -f dist/test-target.raw ] || truncate -s 32G dist/test-target.raw
	@cp -f /usr/share/OVMF/OVMF_VARS_4M.snakeoil.fd dist/test-vars.fd
	qemu-system-x86_64 \
		-machine q35,smm=on \
		-accel kvm \
		-cpu host \
		-smp 4 \
		-m 8G \
		-drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.snakeoil.fd \
		-drive if=pflash,format=raw,unit=1,file=dist/test-vars.fd \
		-cdrom dist/ingot_0.1.0.iso \
		-drive id=disk0,if=none,format=raw,file=dist/test-target.raw \
		-device virtio-blk-pci,drive=disk0 \
		-netdev user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22 \
		-device virtio-net-pci,netdev=net0 \
		-serial mon:stdio \
		-display none \
		{{extra}}

# T7 release pipeline: produces the signed release asset set (root.erofs, UKI,
# ISO, SHA256SUMS, manifest.json, and GPG signatures) with hard 2 GiB assertion.
release *args:
	python3 tools/release.py {{args}}

# T7 publish gate: publishes the release to GitHub Releases, gated on a green
# harness result; immutable (refuses to overwrite existing releases).
publish *args:
	python3 tools/publish.py {{args}}
