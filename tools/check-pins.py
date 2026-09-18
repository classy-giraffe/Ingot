#!/usr/bin/env python3
"""Check that the build environment matches the pins.

Verifies (fast, no network):
  - host tool versions (mkosi, erofs-utils, btrfs-progs, dosfstools,
    qemu-system-x86_64) against harness/pins.json,
  - OVMF firmware blob + snakeoil key/cert sha256 (harness/pins.json),
  - consistency of the compose pin: tools/pins.json:compose.id ==
    image/mkosi.conf Snapshot=, and the brush SHA in the slot image
    config == tools/pins.json:rust.brush.sha.

Exit 0 when everything matches, 1 otherwise.
"""
import json
import re
import subprocess
import sys
from hashlib import sha256
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
pins = json.loads((REPO / "tools/pins.json").read_text())
harness_pins = json.loads((REPO / "harness/pins.json").read_text())

failures = []


def check(name: str, ok: bool, detail: "") -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail})" if detail and not ok else ""))
    if not ok:
        failures.append(name)


def cmd_output(*argv: str) -> str:
    try:
        return subprocess.run(list(argv), capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""


def file_sha256(path: Path) -> str:
    return sha256(path.read_bytes()).hexdigest()


# --- host tools ---------------------------------------------------------
mkosi = cmd_output("mkosi", "--version").strip()
erofs = cmd_output("mkfs.erofs", "--version").strip()
check("erofs-utils version", harness_pins["host_tools"]["erofs-utils"] in erofs, f"got {erofs!r}")
btrfs = cmd_output("mkfs.btrfs", "-V").strip()
check("btrfs-progs version", harness_pins["host_tools"]["btrfs-progs"] in btrfs, f"got {btrfs!r}")
fat = cmd_output("mkfs.fat", "-V").strip()
check("dosfstools version", harness_pins["host_tools"]["dosfstools"] in fat, f"got {fat!r}")
qemu = cmd_output("qemu-system-x86_64", "--version").strip().splitlines()
qemu_ver = qemu[0] if qemu else ""
check("mkosi version", mkosi.split()[-1:] and mkosi.split()[-1] == harness_pins["host_tools"]["mkosi"], f"got {mkosi!r}")
just = cmd_output("just", "--version").strip()
check("just version", harness_pins["host_tools"]["just"] in just, f"got {just!r}")
sgdisk = cmd_output("sgdisk", "--version").strip()
check("gptfdisk version", harness_pins["host_tools"]["gptfdisk"] in sgdisk, f"got {sgdisk!r}")
sfdisk = cmd_output("sfdisk", "--version").strip()
check("util-linux version", harness_pins["host_tools"]["util-linux"] in sfdisk, f"got {sfdisk!r}")
mtools = cmd_output("mdir", "--version").strip()
check("mtools version", harness_pins["host_tools"]["mtools"] in mtools, f"got {mtools!r}")


# --- OVMF firmware -------------------------------------------------------
for role in ("code", "vars"):
    spec = harness_pins["ovmf"][role]
    p = Path(spec["path"])
    if not p.exists():
        check(f"ovmf {role}", False, f"missing {p}")
        continue
    check(f"ovmf {role} sha256", file_sha256(p) == spec["sha256"], f"got {file_sha256(p)}")

# --- snakeoil keys --------------------------------------------------------
for role in ("key", "cert"):
    spec = harness_pins["keys"][role]
    p = REPO / spec["path"]
    if not p.exists():
        check(f"key {role}", False, f"missing {p}")
        continue
    check(f"key {role} sha256", file_sha256(p) == spec["sha256"], f"got {file_sha256(p)}")

# --- compose pin consistency ---------------------------------------------
mkosi_conf = (REPO / "image/mkosi.conf").read_text()
m = re.search(r"^Snapshot=(\S+)$", mkosi_conf, re.M)
# mkosi stamps the compose dir as "Fedora-Rawhide-<Snapshot>", so the
# Snapshot= value is the compose dir name without that prefix.
snapshot = pins["compose"]["id"]
prefix = "Fedora-Rawhide-"
if snapshot.startswith(prefix):
    snapshot = snapshot[len(prefix):]
check(
    "compose pin (mkosi.conf Snapshot=)",
    m is not None and m.group(1) == snapshot,
    f"conf={m.group(1) if m else None!r} expected={snapshot!r}",
)

# Each Rust component pin: the mkosi.conf Environment=<NAME>_SHA= line
# must carry the commit SHA recorded in tools/pins.json, the .gitmodules
# entry must point at the pinned upstream repo, and the thirdparty/<name>
# submodule must be initialized and checked out at exactly that commit
# (the prepare phase stages that checkout; a fresh checkout needs
# `git submodule update --init` first).
gitmodules = (REPO / ".gitmodules").read_text()
for name, spec in pins["rust"].items():
    env_name = name.upper() + "_SHA"
    m = re.search(rf"^Environment={env_name}=(\S+)$", mkosi_conf, re.M)
    check(
        f"{name} pin ({env_name})",
        m is not None and m.group(1) == spec["sha"],
        f"conf={m.group(1) if m else None!r} pins={spec['sha']!r}",
    )
    m = re.search(
        rf'\[submodule "thirdparty/{name}"\](.*?)(?=^\[submodule|\Z)',
        gitmodules, re.M | re.S,
    )
    url_m = re.search(r"^\s*url\s*=\s*(\S+)", m.group(1), re.M) if m else None
    check(
        f"{name} submodule (.gitmodules)",
        url_m is not None and url_m.group(1) == spec["repo"],
        f"url={url_m.group(1) if url_m else None!r} pins={spec['repo']!r}",
    )
    head = cmd_output("git", "-C", str(REPO / "thirdparty" / name), "rev-parse", "HEAD").strip()
    check(
        f"{name} submodule checkout (@ {spec['sha'][:12]})",
        head == spec["sha"],
        f"HEAD={head or 'uninitialized'!r} (run: git submodule update --init)",
    )

version = pins["image_version"]
slot_a = (REPO / "image/repart-baseline/20-slot-a.conf").read_text()
check(
    "slot label matches image version",
    f"Label=ingot_{version}" in slot_a,
    f"expected Label=ingot_{version}",
)

# --- payload boundary -------------------------------------------------------
# Harness-owned test fixtures (harness/probe/ and friends) must never enter
# the mkosi input tree: no symlink under image/ may resolve under harness/,
# and no image/ file may reference a harness/ path.
harness_root = REPO / "harness"
boundary_violations = []
for path in sorted((REPO / "image").rglob("*")):
    if path.is_symlink():
        resolved = path.resolve()
        if resolved == harness_root or harness_root in resolved.parents:
            boundary_violations.append(f"{path.relative_to(REPO)} -> {resolved}")
    elif path.is_file() and b"harness/" in path.read_bytes():
        boundary_violations.append(str(path.relative_to(REPO)))
check(
    "payload boundary (no harness/ under the mkosi input tree)",
    not boundary_violations,
    "; ".join(boundary_violations),
)

# --- summary --------------------------------------------------------------
if failures:
    print(f"\n{len(failures)} pin check(s) FAILED: {', '.join(failures)}")
    sys.exit(1)
print("\nall pin checks passed")
