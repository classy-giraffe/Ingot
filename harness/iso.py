"""iso.py - Ingot live ISO end-to-end harness (issue #23, spec 10.2).

Asserts acceptance criteria 11 and 12 (spec 25.11/25.12):

Scenario A - the working live ISO (criterion 12):
  boot dist/ingot_0.1.0.iso with the snakeoil OVMF (Secure Boot on),
  no config disk. The ISO boots headless (getty on the serial
  console), the live root is the ISO payload (erofs /usr on a tmpfs
  root), the unattended-installer trigger runs (and finds no config
  disk), SSH is reachable (root with the repo's live test key,
  tools/keys/live.key), and the state is ephemeral: a marker file and
  the machine-id from the first boot are gone after a reboot.

Scenario B - the working installer through the live path
  (criterion 11): boot the same ISO with a declarative config disk
  (ingot-install.toml, [source] mode = "live") and a target disk.
  The installer runs unattended and completes; the reboot into the
  target yields the installed machine: the full A/B layout, Secure
  Boot on, /etc materialized from the config (hostname, initial
  user, authorized_keys), and reachable SSH as the initial user.

Host requirements: root (KVM, the config-disk loop mount), the
host tools pinned in harness/pins.json, the dist/ artifacts from
tools/build.sh and the ISO from tools/build-iso.sh.

    sudo python3 harness/iso.py
"""

import hashlib
import json
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

import console
import esp
import gpt
import run

REPO = Path(__file__).resolve().parent.parent
DIST = REPO / "dist"
WORK = DIST / "harness" / "iso"

PINS_VERSION = json.loads((REPO / "tools/pins.json").read_text())["image_version"]
ISO = DIST / f"ingot_{PINS_VERSION}.iso"
LIVE_KEY = REPO / "tools/keys/live.key"

# Scenario A: the live session (root, live test key).
SSH_USER_LIVE = "root"
# Scenario B: the initial user the config disk creates.
SSH_USER_INSTALLED = "harness"

HOSTNAME_LIVE = "ingot-live"
HOSTNAME_INSTALLED = "ingot-harness"

# The live-install markers (image/files/usr/lib/ingot/live-install.sh).
MARK_NO_CONFIG = "INGOT-LIVE-INSTALL: no config disk found"
MARK_CONFIG = "INGOT-LIVE-INSTALL: config disk at"
MARK_COMPLETE = "INGOT-LIVE-INSTALL: complete (installer exit 0)"
MARK_FAILED = "INGOT-LIVE-INSTALL: failed"

# The declarative config disk's TOML (spec 11.4; the workstation
# profile defaults from the wizard). [target] disk is the guest path
# of the target disk (QEMU virtio attach order: target first).
CONFIG_TOML = """\
schema = 1

[target]
disk = "/dev/vda"

[source]
mode = "live"
base = "/media/ingot-iso"

[system]
hostname = "ingot-harness"
timezone = "UTC"
locale = "C.UTF-8"
keymap = "us"

[partitions]
esp = "1G"
slot_a = "8G"
slot_b = "8G"
var = "4G"
home = "8G"

[filesystems]
slot = "erofs"
var = "btrfs"
home = "btrfs"

[encryption]
var = "none"
home = "none"

[[users]]
name = "harness"
shell = "/usr/bin/nushell"

[ssh]
authorized_keys = [
    "{key}",
]

[services]
enabled = []
"""

# The target disk must cover the config's partition total
# (1G + 8G + 8G + 4G + 8G) plus the GPT.
TARGET_DISK_GB = 32
CONFIG_DISK_MB = 32


def _check_key(pins):
    spec = pins["keys"]["live_key"]
    if not LIVE_KEY.is_file():
        sys.exit(f"iso: missing live test key {LIVE_KEY}")
    got = hashlib.sha256(LIVE_KEY.read_bytes()).hexdigest()
    if got != spec["sha256"]:
        sys.exit(f"iso: live test key sha256 mismatch for {LIVE_KEY}")
    pub = (LIVE_KEY.parent / "live.key.pub").read_text().strip()
    parts = pub.split()
    return f"{parts[0]} {parts[1]}"


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def ssh_run(port, user, cmd, timeout=30, retries=24, backoff=5):
    """Run *cmd* in the guest over the hostfwd SSH port.

    Retries while the guest's sshd/network is still coming up (the
    boot marker lands before networkd finishes). Returns
    (rc, stdout, stderr); rc 255 is the ssh-transport failure code.
    """
    args = [
        "ssh",
        "-i", str(LIVE_KEY),
        "-p", str(port),
        f"{user}@127.0.0.1",
        "-o", "BatchMode=yes",
        "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null",
        "-o", f"ConnectTimeout={timeout}",
        "-o", "LogLevel=ERROR",
        cmd,
    ]
    for attempt in range(retries):
        r = subprocess.run(args, capture_output=True, text=True)
        if r.returncode != 255:
            return r.returncode, r.stdout, r.stderr
        if attempt + 1 < retries:
            time.sleep(backoff)
    return r.returncode, r.stdout, r.stderr


class Qemu:
    """One QEMU boot: the snakeoil OVMF (fresh NVRAM per boot), the
    ISO cdrom, optional virtio target/config disks, and an optional
    user-net hostfwd for SSH."""

    def __init__(self, workdir, ovmf, iso=None, disks=(), ssh_port=None):
        workdir = Path(workdir)
        workdir.mkdir(parents=True, exist_ok=True)
        self.console_path = workdir / "console.log"
        self.monitor_sock = workdir / "monitor.sock"
        self.vars_fd = workdir / "vars.fd"
        for f in (self.console_path, self.monitor_sock):
            f.unlink(missing_ok=True)
        shutil.copyfile(ovmf["vars"], self.vars_fd)

        args = [
            "qemu-system-x86_64",
            "-machine", "q35,smm=on",
            "-accel", "kvm",
            "-cpu", "host",
            "-m", "4G",
            "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={ovmf['code']}",
            "-drive", f"if=pflash,format=raw,unit=1,file={self.vars_fd}",
        ]
        if iso:
            args += ["-cdrom", str(iso)]
        for i, d in enumerate(disks):
            args += [
                "-drive", f"id=disk{i},if=none,format=raw,file={d}",
                "-device", f"virtio-blk-pci,drive=disk{i}",
            ]
        net = "user,id=net0"
        if ssh_port:
            net += f",hostfwd=tcp:127.0.0.1:{ssh_port}-:22"
        args += [
            "-netdev", net,
            "-device", "virtio-net-pci,netdev=net0",
            "-serial", f"file:{self.console_path}",
            "-monitor", f"unix:{self.monitor_sock},server,nowait",
            "-display", "none",
            "-no-reboot",
        ]
        self.proc = subprocess.Popen(args, stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL)

    def wait_marker(self, marker, timeout=420, fail_marker=None):
        """Wait for *marker* on the serial console (or *fail_marker*,
        which aborts the wait early). The VM is left running: the
        caller does the post-boot checks and then powerdowns + stops.
        Returns the console text."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            text = (self.console_path.read_text(errors="replace")
                    if self.console_path.exists() else "")
            if console.console_contains(text, marker):
                break
            if fail_marker and console.console_contains(text, fail_marker):
                break
            if self.proc.poll() is not None:
                break
            time.sleep(1)
        return self.console_path.read_text(errors="replace") if \
            self.console_path.exists() else ""

    def powerdown(self):
        run.powerdown(self.monitor_sock)

    def wait_exit(self, timeout=90):
        deadline = time.monotonic() + timeout
        while self.proc.poll() is None and time.monotonic() < deadline:
            time.sleep(1)
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def stop(self):
        self.wait_exit()


def make_config_disk(path, toml, pub_key):
    """A config disk: an ext4 volume carrying ingot-install.toml at
    its root (the declarative config, spec 11.4). Root: loop mount."""
    path = Path(path)
    if path.exists():
        path.unlink()
    with open(path, "wb") as f:
        f.truncate(CONFIG_DISK_MB * 1024 * 1024)
    subprocess.run(["mkfs.ext4", "-q", str(path)], check=True)
    mount = WORK / "config-mount"
    mount.mkdir(parents=True, exist_ok=True)
    subprocess.run(["sudo", "mount", "-o", "loop", str(path), str(mount)],
                   check=True)
    try:
        (mount / "ingot-install.toml").write_text(toml.format(key=pub_key))
        subprocess.run(["sync", str(mount)], check=True)
    finally:
        subprocess.run(["sudo", "umount", str(mount)], check=True)


def scenario_a_live_iso(ovmf):
    """Criterion 12: the ISO boots headless, SB on, SSH reachable,
    state ephemeral across a reboot."""
    checks = {}
    work1 = WORK / "live1"
    work2 = WORK / "live2"

    port1 = _free_port()
    q = Qemu(work1, ovmf, iso=ISO, ssh_port=port1)
    try:
        text1 = q.wait_marker(MARK_NO_CONFIG, timeout=600)
        for _ in range(5):
            if console.console_contains(text1, "login:") or console.console_contains(text1, "getty.target"):
                break
            time.sleep(1)
            text1 = q.console_path.read_text(errors="replace") if q.console_path.exists() else text1
        rc, out, err = ssh_run(port1, SSH_USER_LIVE, "hostname")
        checks["live_ssh"] = {
            "pass": rc == 0 and out.strip() == HOSTNAME_LIVE,
            "detail": f"ssh root@live -> hostname {out.strip()!r} "
                      f"(rc {rc}){err[:120]}",
        }
        checks["live_secure_boot"] = {
            "pass": console.secure_boot_enabled(text1),
            "detail": "Secure boot enabled (console)",
        }
        checks["live_getty"] = {
            "pass": console.console_contains(text1, "getty.target")
            or console.console_contains(text1, "login:"),
            "detail": "Headless serial terminal reached (console)",
        }
        rc, out, err = ssh_run(port1, SSH_USER_LIVE, "cat /proc/mounts")
        mounts = {}
        for line in out.splitlines():
            parts = line.split()
            if len(parts) >= 3:
                mounts[parts[1]] = parts[2]
        checks["live_root"] = {
            "pass": rc == 0 and mounts.get("/") == "tmpfs" and mounts.get("/usr") == "erofs",
            "detail": f"/ {mounts.get('/', 'missing')}, /usr {mounts.get('/usr', 'missing')}",
        }
        rc, machine1, _ = ssh_run(port1, SSH_USER_LIVE, "cat /etc/machine-id")
        ssh_run(port1, SSH_USER_LIVE,
                "mkdir -p /var/tmp && echo harness-marker > /var/tmp/ingot-ephemeral")
    finally:
        q.powerdown()
        q.stop()

    # Reboot the same ISO: the ephemeral state resets.
    port2 = _free_port()
    q2 = Qemu(work2, ovmf, iso=ISO, ssh_port=port2)
    try:
        text2 = q2.wait_marker(MARK_NO_CONFIG, timeout=600)
        rc, machine2, _ = ssh_run(port2, SSH_USER_LIVE, "cat /etc/machine-id")
        rc2, out2, _ = ssh_run(port2, SSH_USER_LIVE,
                               "test -f /var/tmp/ingot-ephemeral && echo present")
        checks["live_ephemeral"] = {
            "pass": (machine1 != machine2 and machine1.strip()
                     and machine2.strip() and out2.strip() != "present"),
            "detail": (f"machine-id {machine1.strip()[:8]}.. -> "
                       f"{machine2.strip()[:8]}..; marker "
                       f"{'present' if out2.strip() == 'present' else 'gone'}"),
        }
    finally:
        q2.powerdown()
        q2.stop()
    return checks, text1


def scenario_b_unattended_install(ovmf):
    """Criterion 11: the config disk auto-triggers the installer on
    the live ISO; the reboot yields the installed machine."""
    checks = {}
    pub = _check_key(run.load_pins())
    work_iso = WORK / "install-iso"
    work_disk = WORK / "install-disk"
    target = WORK / "target.raw"

    # Reuse a prior target disk only when the scenario reruns on a
    # clean slate; a fresh scenario starts from a zeroed disk.
    make_config_disk(WORK / "config.raw", CONFIG_TOML, pub)
    if target.exists():
        target.unlink()
    with open(target, "wb") as f:
        f.truncate(TARGET_DISK_GB * 1024 ** 3)

    port = _free_port()
    q = Qemu(work_iso, ovmf, iso=ISO,
             disks=[target, WORK / "config.raw"], ssh_port=port)
    try:
        text = q.wait_marker(MARK_COMPLETE, timeout=1500, fail_marker=MARK_FAILED)
    finally:
        q.powerdown()
        q.stop()

    checks["install_auto_trigger"] = {
        "pass": console.console_contains(text, MARK_CONFIG)
        and not console.console_contains(text, MARK_FAILED),
        "detail": "config disk found; unattended install started",
    }
    checks["install_complete"] = {
        "pass": console.console_contains(text, MARK_COMPLETE),
        "detail": "installer exit 0 (marker on the serial console)",
    }

    # Host-side layout forensics on the installed disk. Allow disk
    # writes to settle after QEMU shutdown before reading the GPT.
    time.sleep(2)
    g = None
    for _ in range(10):
        try:
            g = gpt.read_gpt(target)
            break
        except gpt.GptError:
            time.sleep(1)
    if g is None:
        g = gpt.read_gpt(target)
    labels = sorted(p.label for p in g.entries)
    expected = sorted(["esp", f"ingot_{PINS_VERSION}", "_empty", "var", "home"])
    checks["installed_layout"] = {
        "pass": labels == expected,
        "detail": f"labels {labels}",
    }
    work_disk.mkdir(parents=True, exist_ok=True)
    esp.carve_esp(target, work_disk / "esp.img")
    forensics = esp.esp_forensics(work_disk / "esp.img")
    want = f"ingot_{PINS_VERSION}"
    checks["installed_esp"] = {
        "pass": want in forensics.get("entries", [])
        and forensics.get("default") == want,
        "detail": f"entries {forensics.get('entries')} default "
                  f"{forensics.get('default')!r}",
    }

    # Boot the installed machine (no ISO): A/B boot, SB on, SSH as
    # the initial user, /etc materialized from the config.
    port2 = _free_port()
    q2 = Qemu(work_disk, ovmf, disks=[target], ssh_port=port2)
    try:
        text2 = q2.wait_marker("Multi-User System", timeout=600)
        rc, out, err = ssh_run(port2, SSH_USER_INSTALLED, "hostname")
        checks["installed_ssh"] = {
            "pass": rc == 0 and out.strip() == HOSTNAME_INSTALLED,
            "detail": f"ssh {SSH_USER_INSTALLED}@installed -> hostname "
                      f"{out.strip()!r} (rc {rc}){err[:120]}",
        }
        rc, out, _ = ssh_run(port2, SSH_USER_INSTALLED, "cat /etc/hostname")
        checks["installed_hostname"] = {
            "pass": rc == 0 and out.strip() == HOSTNAME_INSTALLED,
            "detail": f"/etc/hostname {out.strip()!r} (config materialization)",
        }
        rc, out, _ = ssh_run(port2, SSH_USER_INSTALLED,
                             "sh -c 'id; wc -l < ~/.ssh/authorized_keys'")
        checks["installed_initial_user"] = {
            "pass": rc == 0 and "harness" in out
            and out.strip().splitlines()[-1].strip() == "1",
            "detail": out.replace("\n", " | ")[:140],
        }
        checks["installed_secure_boot"] = {
            "pass": console.secure_boot_enabled(text2),
            "detail": "Secure boot enabled (console)",
        }
        checks["installed_no_failure_markers"] = {
            "pass": not console.failure_markers(text2),
            "detail": "no initramfs failure markers on the console",
        }
    finally:
        q2.powerdown()
        q2.stop()

    return checks, text2


def main():
    pins = run.load_pins()
    ovmf = {"code": pins["ovmf"]["code"]["path"],
            "vars": pins["ovmf"]["vars"]["path"]}
    _check_key(pins)
    if not ISO.is_file():
        sys.exit(f"iso: missing {ISO} (run tools/build-iso.sh)")
    if not shutil.which("qemu-system-x86_64"):
        sys.exit("iso: qemu-system-x86_64 not found")

    WORK.mkdir(parents=True, exist_ok=True)
    print(f"iso: scenario A (live ISO, criterion 12) on {ISO.name}")
    checks_a, _ = scenario_a_live_iso(ovmf)
    for name, c in checks_a.items():
        print(f"  {'PASS' if c['pass'] else 'FAIL'}  {name}  ({c['detail']})")

    print(f"iso: scenario B (unattended install, criterion 11)")
    checks_b, _ = scenario_b_unattended_install(ovmf)
    for name, c in checks_b.items():
        print(f"  {'PASS' if c['pass'] else 'FAIL'}  {name}  ({c['detail']})")

    all_checks = {f"a_{k}": v for k, v in checks_a.items()}
    all_checks.update({f"b_{k}": v for k, v in checks_b.items()})
    results = {
        "mode": "iso-e2e",
        "iso": str(ISO),
        "pass": all(c["pass"] for c in all_checks.values()),
        "checks": all_checks,
    }
    out = WORK / "results.json"
    out.write_text(json.dumps(results, indent=2) + "\n")
    print(f"results: {out}")
    return 0 if results["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
