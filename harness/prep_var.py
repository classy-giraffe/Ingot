"""prep_var.py - inject the harness probe fixture into a disk image's /var.

The probe is a harness-owned fixture (harness/probe/), never part of the
image payload (payload boundary, tools/check-pins.py). Before a scenario
boot the harness drops it onto the /var partition of the working disk:
  /var/lib/etc/systemd/system/ingot-harness-probe.service  (the unit)
  /var/lib/etc/systemd/system/multi-user.target.wants/...  (wants symlink)
  /var/lib/etc/machine-id                                  (a real ID)
  /var/lib/ingot-probe/harness-probe.sh                    (the script)

The seeded machine-id keeps the scenario disk out of systemd's first-
boot flow: without it the first boot would apply the preset policy
(stripping the injected probe's enablement) and run the interactive
systemd-firstboot wizard, which blocks the serial console.

Runs as root (loop mount + mount): the harness invokes it via
passwordless sudo (``sudo python3 harness/prep_var.py``).

Usage: prep_var.py <disk-image>
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import gpt

VAR_LABEL = "var"
UNIT_NAME = "ingot-harness-probe.service"
# Fixed harness-fixture machine-id (deterministic; scenario disks are
# disposable). Must be a valid non-zero 32-hex-digit ID: systemd treats
# an all-zero or missing ID as "uninitialized" (first boot).
MACHINE_ID = "deadbeefdeadbeefdeadbeefdeadbeef"


def inject(root: Path, probe_dir: Path) -> None:
    """Write the probe fixture under the /var partition root (idempotent).

    *root* is the /var partition's mountpoint (the partition itself is
    mounted at /var at runtime), so the fixture lands at the runtime
    paths /var/lib/etc/... and /var/lib/ingot-probe/... - not under a
    stray ``var/`` subdirectory of /var.
    """
    unit_dst = root / "lib/etc/systemd/system" / UNIT_NAME
    wants_dst = (root / "lib/etc/systemd/system"
                 / "multi-user.target.wants" / UNIT_NAME)
    script_dst = root / "lib/ingot-probe/harness-probe.sh"

    unit_dst.parent.mkdir(parents=True, exist_ok=True)
    wants_dst.parent.mkdir(parents=True, exist_ok=True)
    script_dst.parent.mkdir(parents=True, exist_ok=True)

    shutil.copyfile(probe_dir / UNIT_NAME, unit_dst)
    os.chmod(unit_dst, 0o644)
    shutil.copyfile(probe_dir / "harness-probe.sh", script_dst)
    os.chmod(script_dst, 0o755)

    if wants_dst.is_symlink() or wants_dst.exists():
        wants_dst.unlink()
    wants_dst.symlink_to("../" + UNIT_NAME)

    # A real machine-id in the admin layer: systemd's first-boot flow
    # (preset policy + interactive firstboot) must not run on a
    # scenario disk. Set once, never rewritten: the ID is the disk's
    # identity and must survive re-injection (reboots, scenario
    # restarts).
    machine_id_dst = root / "lib/etc/machine-id"
    if not machine_id_dst.exists():
        machine_id_dst.parent.mkdir(parents=True, exist_ok=True)
        machine_id_dst.write_text(MACHINE_ID)
        os.chmod(machine_id_dst, 0o444)


def main(disk: str) -> int:
    if os.geteuid() != 0:
        print("prep_var: must run as root (sudo python3 harness/prep_var.py)",
              file=sys.stderr)
        return 2
    probe_dir = Path(__file__).resolve().parent / "probe"

    g = gpt.read_gpt(disk)
    try:
        var = g.by_label(VAR_LABEL)
    except gpt.GptError as e:
        print(f"prep_var: {disk}: {e}", file=sys.stderr)
        return 1
    off = var.first_lba * gpt.SECTOR
    size = var.size_bytes

    mnt = Path(tempfile.mkdtemp(prefix="ingot-prep-"))
    loop = None
    rc = 1
    try:
        # losetup, not "mount -o loop,...": mount(8) leaks the loop
        # options (size=) into the filesystem option list, and btrfs
        # rejects unknown parameters ("Unknown parameter 'size'").
        r = subprocess.run(
            ["losetup", "--find", "--show",
             f"--offset={off}", f"--size={size}", disk],
            capture_output=True, text=True,
        )
        if r.returncode != 0:
            print(f"prep_var: losetup failed: {r.stderr.strip()}", file=sys.stderr)
            return 1
        loop = r.stdout.strip()
        r = subprocess.run(["mount", "-t", "btrfs", loop, str(mnt)],
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(f"prep_var: mount failed: {r.stderr.strip()}", file=sys.stderr)
            return 1
        try:
            inject(mnt, probe_dir)
            subprocess.run("sync", check=True)
            rc = 0
        finally:
            for arg in ((["umount", str(mnt)]),
                        (["umount", "-l", str(mnt)])):
                r = subprocess.run(arg, capture_output=True, text=True)
                if r.returncode == 0:
                    break
    finally:
        if loop:
            subprocess.run(["losetup", "-d", loop], capture_output=True)
        shutil.rmtree(mnt, ignore_errors=True)
    if rc == 0:
        print(f"prep_var: probe injected into {disk} ({VAR_LABEL} partition)")
    return rc


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
