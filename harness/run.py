"""run.py - Ingot boot harness (T1 gate; boot primitive for T2).

Boots the deployed disk image in QEMU (KVM, pinned OVMF snakeoil
Secure Boot) and asserts the T1 boot invariants. The probe is a
harness-owned fixture (harness/probe/) injected into the working
disk's /var partition before boot (harness/prep_var.py via
passwordless sudo); ``--no-probe``
runs the production gate on host-side evidence only (serial console,
kernel command line, ESP forensics).

Machine-readable results: exit code (0 = all pass) and results.json.
run_ab.py imports ``boot_disk()`` for multi-boot scenarios.

Usage: run.py [--disk PATH] [--no-probe] [--timeout SECS] [--work DIR]
"""

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

import console
import esp

REPO = Path(__file__).resolve().parent.parent
DIST = REPO / "dist"
WORK = DIST / "harness"
# Probe-mode settle: the probe frame is emitted at multi-user.target;
# systemd-bless-boot (the boot bless: it strips the boot counter from
# the ESP entry name) runs shortly after boot-complete.target. Wait
# this long after the frame before powerdown so the post-boot ESP
# forensics observe the blessed state, not a mid-flight powerdown.
BLESS_SETTLE_S = 5


def _grep_uuid(rel):
    for line in (REPO / rel).read_text().splitlines():
        if line.startswith("UUID="):
            return line.split("=", 1)[1]
    raise RuntimeError(f"no UUID= in {rel}")


# Fixed-UUID layout (image/repart-baseline/): the static repart
# definitions are the single source of truth; T1 boots the slot-A disk.
STATE_UUID = _grep_uuid("image/repart-baseline/40-var.conf")
SLOT_A_UUID = _grep_uuid("image/repart-baseline/20-slot-a.conf")
SLOT_B_UUID = _grep_uuid("image/repart-baseline/30-slot-b.conf")
# The pinned release version (tools/pins.json). T1 boots the pinned
# release's slot-A disk; T2 (run_ab.py) deploys the newer version
# into the _empty slot.
PINS_VERSION = json.loads((REPO / "tools/pins.json").read_text())["image_version"]


def load_pins():
    pins = json.loads((REPO / "harness/pins.json").read_text())
    for role in ("code", "vars"):
        spec = pins["ovmf"][role]
        p = Path(spec["path"])
        if not p.is_file():
            sys.exit(f"run: missing firmware {p}")
        got = hashlib.sha256(p.read_bytes()).hexdigest()
        if got != spec["sha256"]:
            sys.exit(f"run: firmware sha256 mismatch for {p}")
    return pins


def powerdown(monitor_sock):
    """Graceful ACPI powerdown via the HMP monitor; True if the monitor
    accepted the command."""
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(5)
        s.connect(str(monitor_sock))
        s.sendall(b"system_powerdown\n")
        time.sleep(0.5)
        s.close()
        return True
    except OSError:
        return False


def boot_disk(disk, workdir, ovmf, timeout=420, mode="probe", marker=None):
    """Boot *disk* (a working disk image; NOT copied here) in a fresh QEMU
    instance with fresh OVMF NVRAM (the loader's EFI variables do not
    leak between boots).

    mode="probe": wait for the harness probe frame on the serial console.
    mode="marker": wait for *marker* (a console line substring).
    Returns an evidence dict: console text, probe document (or None),
    kernel command line, console-based flags, outcome, duration.
    """
    workdir = Path(workdir)
    workdir.mkdir(parents=True, exist_ok=True)
    console_path = workdir / "console.log"
    monitor_sock = workdir / "monitor.sock"
    vars_fd = workdir / "vars.fd"
    for f in (console_path, monitor_sock):
        f.unlink(missing_ok=True)
    shutil.copyfile(ovmf["vars"], vars_fd)

    args = [
        "qemu-system-x86_64",
        "-machine", "q35,smm=on",
        "-accel", "kvm",
        "-cpu", "host",
        "-m", "4G",
        "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={ovmf['code']}",
        "-drive", f"if=pflash,format=raw,unit=1,file={vars_fd}",
        "-drive", f"id=disk,if=none,format=raw,file={disk}",
        "-device", "virtio-blk-pci,drive=disk",
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,netdev=net0",
        "-serial", f"file:{console_path}",
        "-monitor", f"unix:{monitor_sock},server,nowait",
        "-display", "none",
        "-no-reboot",
    ]
    qemu = subprocess.Popen(args, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    t0 = time.monotonic()
    deadline = t0 + timeout
    found = False
    while time.monotonic() < deadline:
        text = console_path.read_text(errors="replace") if console_path.exists() else ""
        if mode == "probe":
            found = console.PROBE_END in text
        else:
            # status lines are ANSI-decorated (the unit ID is wrapped
            # in color codes); match the stripped form
            found = console.console_contains(text, marker)
        if found:
            break
        if qemu.poll() is not None:
            break
        time.sleep(1)

    outcome = ("probe" if mode == "probe" else "marker") if found else "timeout"
    if found:
        if mode == "probe":
            time.sleep(BLESS_SETTLE_S)
        powerdown(monitor_sock)
        deadline_end = time.monotonic() + 60
        while qemu.poll() is None and time.monotonic() < deadline_end:
            time.sleep(1)
    if qemu.poll() is None:
        qemu.terminate()
        try:
            qemu.wait(timeout=15)
        except subprocess.TimeoutExpired:
            qemu.kill()

    text = console_path.read_text(errors="replace") if console_path.exists() else ""
    return {
        "console": text,
        "console_path": str(console_path),
        "probe": console.extract_probe_frame(text),
        "cmdline": console.kernel_cmdline(text),
        "secure_boot": console.secure_boot_enabled(text),
        "reached_multi_user": console.reached_target(text, "multi-user.target"),
        "runtime_root_prepared": console.runtime_root_prepared(text),
        "failure_markers": console.failure_markers(text),
        "outcome": outcome,
        "duration_s": round(time.monotonic() - t0, 1),
    }


def inject_probe(disk, workdir):
    """Drop the probe fixture into the working disk's /var partition."""
    prep = Path(__file__).resolve().parent / "prep_var.py"
    log = Path(workdir) / "prep.log"
    r = subprocess.run(["sudo", "python3", str(prep), str(disk)],
                       capture_output=True, text=True)
    log.write_text(r.stdout + r.stderr)
    if r.returncode != 0:
        sys.exit(f"run: probe injection failed:\n{log.read_text()}")


def guest_secure_boot(p):
    """The guest-side Secure Boot state from the probe document.

    The kernel's own console statement is authoritative; the
    guest-side check depends on what the kernel exposes under
    /sys/firmware/efi: with efivars the SecureBoot efivar is the
    evidence (efivars_sb), with a full efi sysfs the sbs flag
    (secure_boot), and with no efi sysfs at all the kernel console
    statement is the only evidence.
    """
    efi = str(p.get("sysfs_efi", ""))
    if "efivars" in efi:
        return p.get("efivars_sb") == 1
    if efi:
        return p.get("secure_boot") == 1
    return True


def t1_checks(p, ev, esp_state):
    """The T1 boot invariants (probe mode)."""
    checks = {}

    def add(name, ok, detail):
        checks[name] = {"pass": bool(ok), "detail": detail}

    console_text = ev["console"]
    cmdline = p.get("cmdline", "")
    usr = p.get("usr", {})
    var = p.get("var", {})
    etc = p.get("etc", {})

    # 1. UEFI boot with Secure Boot enabled. The kernel's own boot-console
    #    statement is authoritative; the guest-side evidence follows what
    #    the kernel exposes (guest_secure_boot).
    guest_sb = guest_secure_boot(p)
    add("uefi_secure_boot",
        ev["secure_boot"] and guest_sb,
        f"console_sb={ev['secure_boot']} sbs={p.get('secure_boot')} "
        f"efivars_sb={p.get('efivars_sb')} sysfs_efi={p.get('sysfs_efi')!r}")

    # 2. Boot from the slot-A UKI: its kernel command line is the running one
    add("boot_from_uki",
        f"root=PARTUUID={STATE_UUID}" in cmdline
        and "rootfstype=btrfs" in cmdline
        and f"usr=PARTUUID={SLOT_A_UUID}" in cmdline
        and "usrfstype=erofs" in cmdline,
        f"cmdline={cmdline!r}")

    # 2b. / is a tmpfs (the minimal runtime root, CONTEXT.md)
    add("root_tmpfs", p.get("root", {}).get("fstype") == "tmpfs",
        f"root={p.get('root')!r}")

    # 3. systemd as PID 1
    add("systemd_pid1",
        p.get("pid1_comm") == "systemd"
        and str(p.get("pid1_exe", "")).endswith("/lib/systemd/systemd"),
        f"comm={p.get('pid1_comm')!r} exe={p.get('pid1_exe')!r}")

    # 4. /usr mounted read-only from the slot (erofs)
    add("usr_slot_ro",
        usr.get("fstype") == "erofs"
        and "ro" in usr.get("options", "").split(",")
        and SLOT_A_UUID in (usr.get("partuuid", "") + usr.get("source", "")),
        f"usr={usr!r}")

    # 4b. /var is the btrfs state partition (mounted by the initramfs
    #     99ingot module before switch-root)
    add("var_btrfs", var.get("fstype") == "btrfs", f"var={var!r}")

    # 5. /etc present (bind over /var/lib/etc) with materialized fstab
    add("etc_present", etc.get("is_mount") == 1 and etc.get("fstab_nonempty") == 1,
        f"etc={etc!r}")

    # 6. multi-user reached, journal without fatal errors, no failed units
    journal_err = p.get("journal_err", "")
    failed = p.get("failed_units", "").strip()
    add("multi_user_reached", p.get("multi_user") == "active",
        f"multi_user={p.get('multi_user')!r} running={p.get('system_running')!r}")
    add("journal_clean", journal_err.strip() == "" and failed == "",
        f"failed_units={failed!r} journal_err={journal_err[:400]!r}")

    # 7. brush is /bin/sh and system units started under it
    add("brush_sh", p.get("sh") == "/usr/bin/brush", f"sh={p.get('sh')!r}")

    # 8. the booted slot is the pinned release (slot-baked os-release)
    add("slot_version", p.get("version") == PINS_VERSION,
        f"version={p.get('version')!r} want={PINS_VERSION!r}")

    # 9. ESP state after boot: the versioned UKI entry is present and is
    #    the modeled default entry (loader selection state, machine-readable).
    want = f"ingot_{PINS_VERSION}"
    add("esp_entry_present", want in esp_state["entries"],
        f"entries={esp_state['entries']!r}")
    add("esp_default", esp_state["default"] == want,
        f"default={esp_state['default']!r}")

    return checks


def no_probe_checks(ev, esp_state, pins_version):
    """Production gate: host-side evidence only (no guest probe)."""
    checks = {}

    def add(name, ok, detail):
        checks[name] = {"pass": bool(ok), "detail": detail}

    want = f"ingot_{pins_version}"
    add("uefi_secure_boot", ev["secure_boot"], "kernel console statement")
    add("boot_from_uki",
        ev["cmdline"] is not None
        and f"root=PARTUUID={STATE_UUID}" in ev["cmdline"]
        and "rootfstype=btrfs" in ev["cmdline"]
        and f"usr=PARTUUID={SLOT_A_UUID}" in ev["cmdline"]
        and "usrfstype=erofs" in ev["cmdline"],
        f"cmdline={ev['cmdline']!r}")
    add("multi_user_reached", ev["reached_multi_user"], "console target line")
    add("runtime_root_prepared", ev["runtime_root_prepared"],
        "ingot-root.service finished (initramfs prep)")
    add("esp_entry_present", want in esp_state["entries"],
        f"entries={esp_state['entries']!r}")
    add("esp_default", esp_state["default"] == want,
        f"default={esp_state['default']!r}")
    add("no_failure_markers", not ev["failure_markers"],
        f"markers={ev['failure_markers']!r}")
    return checks


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--disk", default=str(DIST / f"ingot_{PINS_VERSION}.raw"))
    ap.add_argument("--no-probe", action="store_true",
                    help="production gate: host-side evidence only")
    ap.add_argument("--timeout", type=int, default=int(os.environ.get("TIMEOUT", "420")))
    ap.add_argument("--work", default=str(WORK))
    args = ap.parse_args()

    pins = load_pins()
    ovmf = {"code": pins["ovmf"]["code"]["path"], "vars": pins["ovmf"]["vars"]["path"]}

    workdir = Path(args.work)
    workdir.mkdir(parents=True, exist_ok=True)
    disk = workdir / "disk.img"
    shutil.copyfile(args.disk, disk)
    print(f"run: booting {args.disk} (work {disk}, timeout {args.timeout}s)")

    if not args.no_probe:
        inject_probe(disk, workdir)

    ev = boot_disk(disk, workdir / "boot", ovmf, timeout=args.timeout,
                   mode="probe" if not args.no_probe else "marker",
                   marker=None if not args.no_probe
                   else "Reached target Multi-User System")

    # post-boot ESP forensics (the disk file holds the post-boot state)
    esp_img = workdir / "esp.img"
    esp.carve_esp(disk, esp_img)
    esp_state = esp.esp_forensics(esp_img)

    if args.no_probe:
        checks = no_probe_checks(ev, esp_state, PINS_VERSION)
    else:
        checks = t1_checks(ev["probe"] or {}, ev, esp_state)
        checks["probe_frame"] = {
            "pass": ev["probe"] is not None and ev["outcome"] == "probe",
            "detail": f"outcome={ev['outcome']} duration={ev['duration_s']}s",
        }

    results = {
        "mode": "no-probe" if args.no_probe else "probe",
        "disk": args.disk,
        "pass": all(c["pass"] for c in checks.values()) and bool(checks),
        "checks": checks,
        "probe": ev["probe"],
        "esp": esp_state,
        "evidence": {k: v for k, v in ev.items() if k != "console"},
    }
    out = workdir / "results.json"
    out.write_text(json.dumps(results, indent=2) + "\n")
    for name, c in checks.items():
        print(f"  {'PASS' if c['pass'] else 'FAIL'}  {name}  ({c['detail']})")
    print(f"results: {out}")
    return 0 if results["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
