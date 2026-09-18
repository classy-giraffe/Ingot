#!/usr/bin/env python3
"""wizard.py - T5 scenario: the interactive TUI wizard (issue #19).

Drives `ingot-installer --wizard` over a pty and verifies the issue's
acceptance criteria on a real disk:

  1. wizard install: full TUI navigation (all 11.4 categories),
     plan shown from the written config, explicit confirmation,
     real engine install on a fresh qcow2; exit 0.
  2. engine rerun: engine mode with the wizard's written config
     (target line pointing at a second fresh qcow2); exit 0.
  3. identity: the two installed disks are identical - GPT layout
     (labels, PARTUUIDs, sizes), slot A partition bytes, ESP
     partition bytes, and the /var and /home trees (modulo
     machine-id, which is seeded fresh per install, 11.5 P6).
  4. abort at the confirmation: esc, y; exit 130; the target file
     is byte-identical (untouched).
  5. abort at the first screen: exit 130.

Machine-readable: one line per check (PASS/FAIL) and a final
summary; exit code 0 only when every check passes.

Run as root (losetup/mount/disk operations), from the repo root:

    sudo python3 harness/wizard.py
    sudo python3 harness/wizard.py --keep      # keep the scratch dirs
"""

import argparse
import fcntl
import hashlib
import json
import os
import pty
import re
import shutil
import struct
import subprocess
import sys
import termios
import threading
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "harness"))
from gpt import read_gpt  # noqa: E402

DISK_GB = 30
VERSION = "0.1.0"
SLOT_LABEL = "ingot_%s" % VERSION


KEYS = {
    "enter": b"\r",
    "esc": b"\x1b",
    "tab": b"\t",
    "backtab": b"\x1b[Z",
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "left": b"\x1b[D",
    "right": b"\x1b[C",
    "backspace": b"\x7f",
}


class Pty:
    """A child process on a pty, modeled as a character grid.

    ratatui's diff renderer writes cells with cursor-position escapes
    (no literal spaces for unchanged cells), so the raw stream is
    replayed into a grid; ``text`` returns the current screen.
    """

    CSI_RE = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z])")

    def __init__(self, argv, cwd, rows=30, cols=120):
        self.rows = rows
        self.cols = cols
        self.grid = [[" "] * cols for _ in range(rows)]
        self.pending_bytes = b""
        self.pending_str = ""
        self.cx = 0
        self.cy = 0
        self.argv = argv
        self.proc = None
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.proc = subprocess.Popen(
            argv,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            cwd=cwd,
            close_fds=True,
        )
        os.close(slave)
        self.master = master
        self.lock = threading.Lock()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        while True:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                break
            if not data:
                break
            with self.lock:
                self._feed(data)

    def _feed(self, data):
        data = self.pending_bytes + data
        self.pending_bytes = b""
        # Decode UTF-8, holding back an incomplete trailing sequence
        # (a multi-byte character split across reads).
        try:
            s = data.decode("utf-8")
        except UnicodeDecodeError:
            s = None
            for k in (1, 2, 3):
                try:
                    s = data[:-k].decode("utf-8")
                    self.pending_bytes = data[-k:]
                    break
                except UnicodeDecodeError:
                    continue
            if s is None:
                s = data.decode("utf-8", "replace")
        s = self.pending_str + s
        self.pending_str = ""
        i = 0
        n = len(s)
        while i < n:
            ch = s[i]
            if ch == "\x1B":
                m = self.CSI_RE.match(s, i)
                if m:
                    params = m.group(1)
                    final = m.group(2)
                    if final == "H":
                        parts = params.split(";")
                        row = int(parts[0]) if parts[0].isdigit() else 1
                        col = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 1
                        self.cy = max(0, min(row - 1, self.rows - 1))
                        self.cx = max(0, min(col - 1, self.cols - 1))
                    elif final == "J":
                        self.grid = [[" "] * self.cols for _ in range(self.rows)]
                        self.cx = self.cy = 0
                    i = m.end()
                    continue
                # Incomplete escape at the chunk tail: keep it for the
                # next chunk (a CSI sequence split across reads would
                # otherwise leave literal "[row;colH" garbage in the
                # grid).
                if s[i + 1 : i + 2] == "[":
                    j = i + 2
                    while j < n and not 0x40 <= ord(s[j]) <= 0x7E:
                        j += 1
                    if j >= n:
                        self.pending_str = s[i:]
                        break
                    i = j + 1  # unusual parameters: skip the sequence
                    continue
                if i == n - 1:
                    self.pending_str = s[i:]
                    break
                i += 1  # bare ESC: drop
                continue
            if ch == "\t":
                ch = " "
            if ord(ch) >= 0x20:
                if self.cx < self.cols:
                    self.grid[self.cy][self.cx] = ch
                self.cx += 1
                if self.cx >= self.cols:
                    self.cx = 0
                    self.cy = min(self.cy + 1, self.rows - 1)
            i += 1

    def text(self):
        with self.lock:
            lines = ["".join(row).rstrip() for row in self.grid]
            while lines and not lines[-1]:
                lines.pop()
            return "\n".join(lines)

    def wait_for(self, needle, timeout=30, interval=0.05):
        """Poll the grid until the needle appears (or the timeout)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if needle in self.text():
                return True
            time.sleep(interval)
        return False

    def send(self, data):
        os.write(self.master, data)

    def key(self, name):
        self.send(KEYS[name])

    def type_field(self, text, clear=20):
        """Replace the selected field's content: clear it, then type."""
        self.send(b"\x7f" * clear)
        self.send(text.encode())

    def finish(self, timeout=30):
        """Wait for the child to exit; return (exit_code, last screen)."""
        try:
            rc = self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            rc = self.proc.wait()
        self.reader.join(timeout=5)
        try:
            os.close(self.master)
        except OSError:
            pass
        return rc, self.text()


class Checks:
    def __init__(self):
        self.rows = []

    def check(self, name, ok, detail=""):
        self.rows.append((name, ok, detail))
        print(
            "  %s %-38s %s" % ("PASS" if ok else "FAIL", name, detail),
            flush=True,
        )

    def failed(self):
        return [r for r in self.rows if not r[1]]


def run_cmd(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def sha256_file(path, chunk=1 << 20):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(chunk)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def region_sha(path, first_lba, last_lba, chunk=1 << 20):
    """SHA-256 of a GPT partition's byte range in a raw image."""
    start = first_lba * 512
    end = (last_lba + 1) * 512
    h = hashlib.sha256()
    with open(path, "rb") as f:
        f.seek(start)
        remaining = end - start
        while remaining > 0:
            b = f.read(min(chunk, remaining))
            if not b:
                break
            h.update(b)
            remaining -= len(b)
    return h.hexdigest()


def extract_partition(raw, img, first_lba, last_lba):
    """Writes a partition's bytes out of a raw image to a file."""
    start = first_lba * 512
    end = (last_lba + 1) * 512
    with open(raw, "rb") as src, open(img, "wb") as dst:
        src.seek(start)
        remaining = end - start
        while remaining > 0:
            b = src.read(min(1 << 20, remaining))
            if not b:
                break
            dst.write(b)
            remaining -= len(b)


def mount(img, mountpoint):
    dev = run_cmd(["losetup", "-f", "--show", img]).stdout.strip()
    ok = run_cmd(["mount", dev, mountpoint]).returncode == 0
    return dev if ok else None


def umount(mountpoint, dev):
    run_cmd(["umount", mountpoint])
    if dev:
        run_cmd(["losetup", "-d", dev])


def qcow2_create(path, gb):
    run_cmd(["qemu-img", "create", "-f", "qcow2", path, "%dG" % gb])


def qcow2_to_raw(qcow2, raw):
    run_cmd(["qemu-img", "convert", "-f", "qcow2", "-O", "raw", qcow2, raw])


# ---------------------------------------------------------------- wizard run


def wizard_argv(config_path, work_dir):
    return [
        os.path.join(REPO, "src/target/release/ingot-installer"),
        "--wizard",
        "--config",
        config_path,
        "--work",
        work_dir,
    ]


def navigate_to_plan(t, disk_path, dist, log=print):
    """Drives the wizard from the first screen to the plan screen."""
    # 1. target disk
    if not t.wait_for("target disk (11.4.1)", 30):
        return False
    t.type_field(disk_path, clear=0)
    t.key("enter")
    # 2. source (base row 0, version row 1)
    if not t.wait_for("OS image source (11.4.2)", 30):
        return False
    t.type_field(dist)
    t.key("down")
    t.type_field(VERSION)
    t.key("enter")
    # 3. system identity: set the hostname, keep the defaults
    if not t.wait_for("system identity (11.4.3-5)", 30):
        return False
    t.type_field("ingot-wiz")
    t.key("enter")
    # 4. partitions (defaults: 1G/8G/8G/4G/8G)
    if not t.wait_for("partition sizes (11.4.6)", 30):
        return False
    t.key("enter")
    # 5. filesystems (erofs/btrfs/btrfs) and 6. encryption (none/none)
    if not t.wait_for("filesystem choices (11.4.7)", 30):
        return False
    t.key("enter")
    if not t.wait_for("encryption choices (11.4.8)", 30):
        return False
    t.key("enter")
    # 7. users (admin), 8. ssh keys (none), 9. services (none)
    if not t.wait_for("initial users (11.4.9)", 30):
        return False
    t.key("enter")
    if not t.wait_for("SSH authorized keys (11.4.10)", 30):
        return False
    t.key("enter")
    if not t.wait_for("service enablement (11.4.11)", 30):
        return False
    t.key("enter")
    # 10. review: must be clean
    if not t.wait_for("validation: no errors", 60):
        log("review screen did not validate; transcript tail:")
        log(t.text()[-2000:])
        return False
    t.key("enter")
    # plan from the written config
    if not t.wait_for("plan ok - config:", 120):
        log("plan did not complete; transcript tail:")
        log(t.text()[-2000:])
        return False
    return True


def navigate_to_confirm(t, disk_path, dist, log=print):
    """Drives the wizard from the first screen to the confirmation."""
    if not navigate_to_plan(t, disk_path, dist, log):
        return False
    t.key("enter")
    if not t.wait_for("WARNING: the install destroys", 30):
        return False
    return True


def abort(t, timeout=15):
    """esc to the abort prompt, then y; return (exit_code, last screen).

    Retries the esc: a lone ESC byte coalesced with the previous key's
    bytes parses as one pair (terminal ESC disambiguation), so a missed
    prompt means the esc was lost.
    """
    for _ in range(3):
        t.key("esc")
        if t.wait_for("Abort the install?", timeout):
            break
    t.send(b"y")
    return t.finish(60)


def scenario_wizard_install(scratch, checks):
    print("scenario: wizard install (full TUI, real engine)", flush=True)
    disk1 = os.path.join(scratch, "disk1.qcow2")
    work1 = os.path.join(scratch, "work-wizard")
    config = os.path.join(scratch, "wizard.toml")
    qcow2_create(disk1, DISK_GB)

    t = Pty(wizard_argv(config, work1), cwd=REPO)
    ok = navigate_to_confirm(t, disk1, os.path.join(REPO, "dist"))
    code, out = (None, "")
    if ok:
        t.send(b"yes")
        t.key("enter")
        if t.wait_for("install complete", 1500):
            t.key("enter")
            code, out = t.finish(60)
        else:
            code, out = t.finish(60)
            ok = False
            print("  install did not complete; transcript tail:")
            print(out[-2500:])
    else:
        code, out = t.finish(30)

    checks.check(
        "wizard: exit 0 after confirmed install", code == 0, "exit=%s" % code
    )
    checks.check(
        "wizard: summary printed after the TUI",
        "install complete" in out and "config: %s" % config in out,
    )
    # the wizard wrote the config the engine runs
    has_config = os.path.exists(config)
    text = open(config).read() if has_config else ""
    checks.check(
        "wizard: wrote the TOML config (all 11.4 categories)",
        has_config
        and all(
            k in text
            for k in (
                "schema = 1",
                "[target]",
                "[source]",
                "[system]",
                "[partitions]",
                "[filesystems]",
                "[encryption]",
                "[[users]]",
                "[ssh]",
                "[services]",
            )
        ),
    )
    checks.check(
        "wizard: config carries the collected values",
        ('disk = "%s"' % disk1) in text
        and ('hostname = "ingot-wiz"') in text
        and ('version = "%s"' % VERSION) in text,
    )
    return disk1, config, out


def scenario_engine_rerun(scratch, disk1, config, checks):
    print("scenario: engine mode with the wizard's config", flush=True)
    disk2 = os.path.join(scratch, "disk2.qcow2")
    work2 = os.path.join(scratch, "work-engine")
    qcow2_create(disk2, DISK_GB)
    cfg2 = os.path.join(scratch, "wizard-disk2.toml")
    text = open(config).read().replace(disk1, disk2, 1)
    open(cfg2, "w").write(text)

    o = run_cmd(
        [
            os.path.join(REPO, "src/target/release/ingot-installer"),
            cfg2,
            "--work",
            work2,
        ],
        cwd=REPO,
    )
    checks.check(
        "engine: rerun with the config exits 0",
        o.returncode == 0,
        "exit=%s" % o.returncode,
    )
    if o.returncode != 0:
        print(o.stderr[-1500:])
    return disk2


def scenario_identity(scratch, disk1, disk2, checks):
    print("scenario: identical installs (wizard vs engine mode)", flush=True)
    raw1 = os.path.join(scratch, "disk1.raw")
    raw2 = os.path.join(scratch, "disk2.raw")
    qcow2_to_raw(disk1, raw1)
    qcow2_to_raw(disk2, raw2)

    g1 = read_gpt(raw1)
    g2 = read_gpt(raw2)
    layout1 = sorted(
        (e.label, e.part_uuid, e.size_bytes)
        for e in g1.entries
        if e.label
    )
    layout2 = sorted(
        (e.label, e.part_uuid, e.size_bytes)
        for e in g2.entries
        if e.label
    )
    checks.check(
        "layout: same partitions (labels, PARTUUIDs, sizes)",
        layout1 == layout2 and len(layout1) == 5,
        "%d partitions" % len(layout1),
    )

    slot1 = g1.by_label(SLOT_LABEL)
    slot2 = g2.by_label(SLOT_LABEL)
    s1 = region_sha(raw1, slot1.first_lba, slot1.last_lba)
    s2 = region_sha(raw2, slot2.first_lba, slot2.last_lba)
    checks.check("slot A: partition bytes identical", s1 == s2, s1[:16] + "...")

    esp1 = g1.by_label("esp")
    esp2 = g2.by_label("esp")
    e1 = region_sha(raw1, esp1.first_lba, esp1.last_lba)
    e2 = region_sha(raw2, esp2.first_lba, esp2.last_lba)
    checks.check("ESP: partition bytes identical", e1 == e2, e1[:16] + "...")

    for part in ("var", "home"):
        img1 = os.path.join(scratch, "%s1.img" % part)
        img2 = os.path.join(scratch, "%s2.img" % part)
        p1 = g1.by_label(part)
        p2 = g2.by_label(part)
        extract_partition(raw1, img1, p1.first_lba, p1.last_lba)
        extract_partition(raw2, img2, p2.first_lba, p2.last_lba)
        mp1 = os.path.join(scratch, "mnt-%s1" % part)
        mp2 = os.path.join(scratch, "mnt-%s2" % part)
        os.makedirs(mp1, exist_ok=True)
        os.makedirs(mp2, exist_ok=True)
        d1 = mount(img1, mp1)
        d2 = mount(img2, mp2)
        checks.check("%s: partition mounts" % part, bool(d1) and bool(d2))
        if d1 and d2:
            # install.log is a per-run event stream (timestamps, loop
            # device names in details) - compared by event sequence below.
            exclude = []
            if part == "var":
                exclude = ["--exclude=machine-id", "--exclude=install.log"]
            o = run_cmd(["diff", "-r"] + exclude + [mp1, mp2])
            checks.check(
                "%s: tree identical%s"
                % (
                    part,
                    " (modulo machine-id and the per-run install.log)" if part == "var" else "",
                ),
                o.returncode == 0,
                o.stdout[:200],
            )
            if part == "var":
                log1 = os.path.join(mp1, "lib/ingot/install.log")
                log2 = os.path.join(mp2, "lib/ingot/install.log")
                ok = os.path.exists(log1) and os.path.exists(log2)
                seq_same = False
                detail = "missing log file"
                if ok:

                    def seq(path):
                        return [
                            (l.get("phase"), l.get("event"))
                            for l in map(json.loads, open(path).read().splitlines())
                            if l
                        ]

                    s1, s2 = seq(log1), seq(log2)
                    seq_same = s1 == s2 and s1[-1] == ("finalize", "install-complete")
                    detail = "%d events each, same sequence: %s" % (len(s1), seq_same)
                checks.check(
                    "var: install.log event sequences identical (same engine run)",
                    ok and seq_same,
                    detail,
                )
            umount(mp1, d1)
            umount(mp2, d2)


def scenario_abort_at_confirm(scratch, checks):
    print("scenario: abort at the confirmation (11.6.2)", flush=True)
    disk = os.path.join(scratch, "disk3.qcow2")
    work = os.path.join(scratch, "work-abort")
    qcow2_create(disk, DISK_GB)
    before = sha256_file(disk)

    t = Pty(wizard_argv(os.path.join(scratch, "abort.toml"), work), cwd=REPO)
    ok = navigate_to_confirm(t, disk, os.path.join(REPO, "dist"))
    code, out = abort(t)
    if not ok:
        checks.check("abort-at-confirm: reached the confirmation", False, out[-400:])

    checks.check(
        "abort: exit 130 (user abort)", code == 130, "exit=%s" % code
    )
    checks.check(
        "abort: target file byte-identical (untouched)",
        sha256_file(disk) == before,
    )
    checks.check(
        "abort: aborted notice on the terminal",
        "wizard aborted" in out,
    )
    checks.check(
        "abort: config written at the review gate stays (target untouched)",
        os.path.exists(os.path.join(scratch, "abort.toml")),
    )


def scenario_abort_early(scratch, checks):
    print("scenario: abort at the first screen", flush=True)
    disk = os.path.join(scratch, "disk4.qcow2")
    work = os.path.join(scratch, "work-abort2")
    qcow2_create(disk, DISK_GB)
    before = sha256_file(disk)

    t = Pty(wizard_argv(os.path.join(scratch, "abort2.toml"), work), cwd=REPO)
    if t.wait_for("target disk (11.4.1)", 30):
        code, out = abort(t)
    else:
        code, out = t.finish(30)
    checks.check("abort-early: exit 130", code == 130, "exit=%s" % code)
    checks.check(
        "abort-early: target file byte-identical (untouched)",
        sha256_file(disk) == before,
    )
    checks.check(
        "abort-early: no config file written",
        not os.path.exists(os.path.join(scratch, "abort2.toml")),
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--keep", action="store_true", help="keep the scratch dir")
    ap.add_argument("--scratch", default=os.path.join(REPO, "t5"))
    args = ap.parse_args()

    if os.geteuid() != 0:
        print("wizard.py needs root (losetup/mount); run: sudo python3 harness/wizard.py")
        return 2
    installer = os.path.join(REPO, "src/target/release/ingot-installer")
    if not os.path.exists(installer):
        print("installer release binary missing: build first (cd src && cargo build --release -p ingot-installer)")
        return 2
    if os.path.exists(args.scratch):
        shutil.rmtree(args.scratch)
    os.makedirs(args.scratch)

    checks = Checks()
    try:
        disk1, config, _out = scenario_wizard_install(args.scratch, checks)
        disk2 = scenario_engine_rerun(args.scratch, disk1, config, checks)
        scenario_identity(args.scratch, disk1, disk2, checks)
        scenario_abort_at_confirm(args.scratch, checks)
        scenario_abort_early(args.scratch, checks)
    finally:
        if args.keep:
            print("scratch kept: %s" % args.scratch)
        else:
            for d in ("mnt-var1", "mnt-var2", "mnt-home1", "mnt-home2"):
                mp = os.path.join(args.scratch, d)
                if os.path.ismount(mp):
                    run_cmd(["umount", mp])
            shutil.rmtree(args.scratch, ignore_errors=True)

    failed = checks.failed()
    print()
    if failed:
        print("FAIL: %d check(s) failed: %s" % (len(failed), [r[0] for r in failed]))
        return 1
    print("PASS: %d checks" % len(checks.rows))
    return 0


if __name__ == "__main__":
    sys.exit(main())
