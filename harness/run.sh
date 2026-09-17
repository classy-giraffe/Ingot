#!/bin/bash
# Ingot boot harness (T1 gate): boots the deployed disk in QEMU with KVM +
# pinned OVMF (snakeoil Secure Boot keys), the probe injected into the
# working disk's /var, and asserts the boot invariants. The harness lives
# in harness/run.py (probe-injection mode by default; --no-probe is the
# host-side production gate). Machine-readable results: exit code
# (0 = all pass) and dist/harness/results.json.
#
# Usage: harness/run.sh [--disk PATH] [--no-probe] [--timeout SECS]
set -euo pipefail

exec python3 "$(dirname "$0")/run.py" "$@"
