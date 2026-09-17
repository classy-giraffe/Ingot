#!/bin/bash
# One-time root setup for the Ingot build on this workstation.
#
# sudo-rs (Rust sudo) on this machine:
#   - cannot match sudoers commands that are text scripts,
#   - parses each sudoers line as ONE command plus its arguments
#     (no multi-command lists).
# The mkosi build needs root (the unprivileged user-namespace sandbox is
# blocked by the kernel LSM). So the build runs through
# /usr/local/bin/ingot-build, an ELF wrapper around tools/build.sh
# (source: tools/ingot-build.c).
#
# Run once as the build user; it asks for the sudo password itself.
# Idempotent.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO=$(pwd)
USER_ID=$(id -un)

cc -O2 -DREPO_ROOT="\"$REPO\"" -o tools/ingot-build tools/ingot-build.c
echo "compiled tools/ingot-build (repo root $REPO)"

sudo bash -c "
    set -eu
    install -m 0755 '$REPO/tools/ingot-build' /usr/local/bin/ingot-build
    {
        echo '$USER_ID ALL=(root) NOPASSWD: /usr/local/bin/ingot-build'
        echo '$USER_ID ALL=(root) NOPASSWD: /usr/bin/systemd-repart'
    } > /etc/sudoers.d/ingot-build
    chmod 0440 /etc/sudoers.d/ingot-build
    visudo -c -q -f /etc/sudoers.d/ingot-build
"
echo "setup: done"
