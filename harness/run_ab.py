"""run_ab.py - T2 scenario: A/B selection and automatic rollback.

Simulates the sysupdate A/B flow against the real boot machinery
(systemd-boot + boot counters + systemd-bless-boot), five boots:

  deploy   build v0.2.0 (if needed) into the _empty slot B of the
           v0.1.0 disk: copy the slot erofs, rename the GPT label,
           install the v0.2.0 UKI on the ESP armed with the standard
           initial counter (ingot_0.2.0+3.efi, TriesLeft=3).
  boot 1   bless: the loader selects v0.2.0 (newest, non-bad); the
           successful boot resets the counters (bless-boot strips the
           counter -> the entry is good).
  fails    a later update is bad: re-arm the entry (new update attempt,
           TriesLeft=3) and corrupt the v0.2.0 slot; three consecutive
           failed boots decrement the counter (+2-1, +1-2, +0-3).
  boot 5   fallback: the exhausted entry is bad (tries-left 0) and is
           not selected; the loader falls back to v0.1.0, which boots.

Machine-readable results: exit code (0 = all pass) and
dist/harness/ab-results.json (per-boot records + the boot selection
state - default entry, counters, bless state - at every phase).

Usage: run_ab.py [--force-build]
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import console
import esp
import gpt
import run

REPO = Path(__file__).resolve().parent.parent
DIST = REPO / "dist"
WORK = DIST / "harness" / "ab"
# Scenario versions and slot PARTUUIDs (re-exported from run.py's
# fixed-UUID layout). A/B = the two newest versions: V1 is the pinned
# release (slot A, the base disk), V2 is the new version the scenario
# deploys into the _empty slot B.
V1 = run.PINS_VERSION
V2 = "0.2.0"
SLOT_A = run.SLOT_A_UUID
SLOT_B = run.SLOT_B_UUID
# Initial "tries left" counter armed on the freshly deployed slot's UKI
# entry name; the loader decrements it on each failed boot.
ARM_TRIES = 3
# Console evidence of an injected slot failure: the runtime-root prep
# unit (initramfs) fails to mount the corrupted slot. The manager's
# status line is the console-native signal - the initramfs journal does
# not forward service output (prepare-root.sh stderr) to the console.
FAIL_MARKER = "Failed to start ingot-root.service"

ZERO_BYTES = 1 << 20  # erofs superblock at offset 1024; 1MiB covers it


def log(msg):
    print(f"ab: {msg}", flush=True)




def region_sha(disk, entry):
    """sha256 of a partition's contents (re-read from the disk: exactly
    the partition's LBA range, not the rest of the image)."""
    off = entry.first_lba * gpt.SECTOR
    remaining = entry.size_bytes
    h = hashlib.sha256()
    with open(disk, "rb") as f:
        f.seek(off)
        while remaining > 0:
            chunk = f.read(min(1 << 24, remaining))
            if not chunk:
                break
            h.update(chunk)
            remaining -= len(chunk)
    return h.hexdigest()


def write_region(disk, entry, data_path):
    """Overwrite a partition's contents with *data_path* (exact size)."""
    off = entry.first_lba * gpt.SECTOR
    size = os.path.getsize(data_path)
    if size != entry.size_bytes:
        sys.exit(f"ab: artifact size {size} != partition size {entry.size_bytes}")
    with open(disk, "r+b") as f:
        f.seek(off)
        with open(data_path, "rb") as src:
            while True:
                chunk = src.read(1 << 26)
                if not chunk:
                    break
                f.write(chunk)
        f.flush()
        os.fsync(f.fileno())


def zero_region(disk, entry, nbytes):
    """Zero the first *nbytes* of a partition (failure injection: the
    erofs superblock is destroyed, the slot no longer mounts)."""
    off = entry.first_lba * gpt.SECTOR
    with open(disk, "r+b") as f:
        f.seek(off)
        f.write(b"\0" * nbytes)
        f.flush()
        os.fsync(f.fileno())


def set_label(disk, part_uuid, label):
    g = gpt.read_gpt(disk)
    n = g.entries.index(g.by_part_uuid(part_uuid)) + 1
    r = subprocess.run(["sgdisk", "-c", f"{n}:{label}", str(disk)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"ab: sgdisk failed: {r.stderr.strip()}")


def ensure_artifact(version, slot, force):
    """Build (if needed) the version/slot artifacts; return metadata."""
    disk = DIST / f"ingot_{version}.raw"
    slot_raw = DIST / f"ingot_{version}.slot.raw"
    uki = DIST / f"ingot_{version}.efi"
    meta_path = DIST / f"build-metadata-{version}.json"
    need = force or not all(p.is_file() for p in (disk, slot_raw, uki, meta_path))
    if not need and version == V1:
        # a pre-T2 disk carries the kernel-version UKI name, not the
        # versioned entry name the A/B flow needs
        img = WORK / f"check-{version}.esp.img"
        esp.carve_esp(disk, img)
        if f"ingot_{version}" not in esp.esp_files(img):
            need = True
    if need:
        log(f"building v{version} (slot {slot}) - this takes ~15-20 min")
        r = subprocess.run(
            ["tools/build.sh", "--local", "--version", version, "--slot", slot],
            cwd=REPO)
        if r.returncode != 0:
            sys.exit(f"ab: build of v{version} failed")
    return json.loads(meta_path.read_text())


def esp_with(disk, workdir, fn):
    """Carve the ESP, apply fn(carved_img), restore it."""
    img = workdir / "esp.img"
    esp.carve_esp(disk, img)
    fn(img)
    esp.restore_esp(disk, img)


def selection(disk, workdir):
    workdir = Path(workdir)
    workdir.mkdir(parents=True, exist_ok=True)
    img = workdir / "sel-esp.img"
    esp.carve_esp(disk, img)
    return esp.esp_forensics(img)


def boot_checks(name, want_version, want_slot, ev, expect_counter=None):
    """Per-boot assertions. Returns {name: {pass, detail}}."""
    checks = {}

    def add(n, ok, detail):
        checks[n] = {"pass": bool(ok), "detail": detail}

    p = ev["probe"] or {}
    if want_version is None:
        # failure boot: the probe must be absent, the failure on the
        # console, and the loader must have decremented the armed counter
        failed_marker = console.console_contains(ev["console"], FAIL_MARKER)
        add("boot_failed",
            ev["probe"] is None and failed_marker,
            f"probe={'yes' if ev['probe'] else 'no'} "
            f"marker={'yes' if failed_marker else 'no'}")
        add("no_multi_user", not ev["reached_multi_user"],
            f"reached_multi_user={ev['reached_multi_user']}")
        if expect_counter is not None:
            left, attempts = expect_counter
            want = f"ingot_{V2}+{left}-{attempts}"
            got = (ev.get("selection_after") or {}).get("counters", {}).get(want)
            add("counter", got == [left, attempts],
                f"want {want} -> {got} got {ev.get('selection_after', {}).get('entries')}")
        return checks

    add("probe_frame", ev["probe"] is not None and ev["outcome"] == "probe",
        f"outcome={ev['outcome']} duration={ev['duration_s']}s")
    add("version", p.get("version") == want_version,
        f"version={p.get('version')!r} want={want_version!r}")
    usr = p.get("usr", {})
    add("slot", want_slot in (usr.get("partuuid", "") + usr.get("source", "")),
        f"usr={usr!r}")
    # Secure Boot evidence: the kernel console statement plus the
    # guest-side check (shared with the T1 gate: run.guest_secure_boot).
    guest_sb = run.guest_secure_boot(p)
    add("secure_boot", ev["secure_boot"] and guest_sb,
        f"console={ev['secure_boot']} sbs={p.get('secure_boot')} "
        f"efivars_sb={p.get('efivars_sb')}")
    add("multi_user", p.get("multi_user") == "active",
        f"multi_user={p.get('multi_user')!r}")
    add("journal_clean",
        (p.get("journal_err") or "").strip() == ""
        and (p.get("failed_units") or "").strip() == "",
        f"failed={p.get('failed_units')!r} crit={(p.get('journal_err') or '')[:200]!r}")
    return checks


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--force-build", action="store_true",
                    help="rebuild both versions even if artifacts exist")
    args = ap.parse_args()

    WORK.mkdir(parents=True, exist_ok=True)
    pins = run.load_pins()
    ovmf = {"code": pins["ovmf"]["code"]["path"], "vars": pins["ovmf"]["vars"]["path"]}

    results = {"pass": False, "acceptance": {}, "phases": {}, "boots": [],
               "artifacts": {}, "work": str(WORK)}

    # --- deploy ----------------------------------------------------------------
    m1 = ensure_artifact(V1, "a", args.force_build)
    m2 = ensure_artifact(V2, "b", args.force_build)
    results["artifacts"] = {
        V1: {"slot_sha": m1["artifacts"]["slot_erofs"]["sha256"],
             "disk": f"ingot_{V1}.raw"},
        V2: {"slot_sha": m2["artifacts"]["slot_erofs"]["sha256"],
             "disk": f"ingot_{V2}.raw"},
    }

    log("assembling the A/B working disk (v0.1.0 base + v0.2.0 in slot B)")
    disk = WORK / "disk.img"
    shutil.copyfile(DIST / f"ingot_{V1}.raw", disk)
    slot_raw = DIST / f"ingot_{V2}.slot.raw"

    g = gpt.read_gpt(disk)
    slot_b = g.by_part_uuid(SLOT_B)
    write_region(disk, slot_b, slot_raw)
    written_sha = region_sha(disk, gpt.read_gpt(disk).by_part_uuid(SLOT_B))
    if written_sha != m2["artifacts"]["slot_erofs"]["sha256"]:
        sys.exit("ab: slot B contents do not match the v0.2.0 slot artifact")
    set_label(disk, SLOT_B, f"ingot_{V2}")
    labels = [e.label for e in gpt.read_gpt(disk).entries]
    log(f"slot B written (sha verified), labels: {labels}")

    def install_uki(img):
        esp.esp_write(img, f"EFI/Linux/ingot_{V2}+{ARM_TRIES}.efi",
                      (DIST / f"ingot_{V2}.efi").read_bytes())

    esp_with(disk, WORK, install_uki)
    run.inject_probe(disk, WORK)

    deploy = selection(disk, WORK / "deploy")
    results["phases"]["deploy"] = deploy
    if f"ingot_{V2}+{ARM_TRIES}" not in deploy["entries"] \
            or f"ingot_{V1}" not in deploy["entries"]:
        sys.exit(f"ab: deploy ESP state wrong: {deploy['entries']!r}")
    log(f"deploy selection: default={deploy['default']} states={deploy['states']}")

    # --- the five boots ----------------------------------------------------------
    boots = []

    def do_boot(i, name, want_version, want_slot, mode, marker=None,
                expect_counter=None):
        log(f"boot {i} ({name})...")
        ev = run.boot_disk(disk, WORK / f"boot-{i}", ovmf,
                           mode=mode, marker=marker)
        post = selection(disk, WORK / f"post-{i}")
        ev["selection_after"] = post
        checks = boot_checks(name, want_version, want_slot, ev,
                             expect_counter=expect_counter)
        rec = {"boot": i, "phase": name, "checks": checks,
               "outcome": ev["outcome"], "duration_s": ev["duration_s"],
               "selection_after": post,
               "probe_version": (ev["probe"] or {}).get("version"),
               "failure_markers": ev["failure_markers"]}
        boots.append(rec)
        for cname, c in checks.items():
            print(f"  {'PASS' if c['pass'] else 'FAIL'}  {name}.{cname}  ({c['detail']})")
        return rec, ev, post

    rec1, ev1, post1 = do_boot(1, "bless", V2, SLOT_B, "probe")
    results["phases"]["post-bless"] = post1
    blessed = (f"ingot_{V2}" in post1["entries"]
               and post1["states"].get(f"ingot_{V2}") == "good"
               and all("+" not in n for n in post1["entries"]
                       if n.startswith(f"ingot_{V2}")))

    # --- failure injection: re-arm the entry, corrupt slot B ----------------------
    def re_arm(img):
        esp.esp_rename(img, f"EFI/Linux/ingot_{V2}.efi",
                       f"EFI/Linux/ingot_{V2}+{ARM_TRIES}.efi")

    esp_with(disk, WORK, re_arm)
    zero_region(disk, gpt.read_gpt(disk).by_part_uuid(SLOT_B), ZERO_BYTES)
    log(f"failure armed: entry re-armed to +{ARM_TRIES}, slot B zeroed (1MiB)")

    # loader decrement per failed boot: +3 -> +2-1 -> +1-2 -> +0-3 (bad)
    fails = []
    for i in (2, 3, 4):
        rec, ev, post = do_boot(i, f"fail-{i - 1}", None, None,
                                "marker", marker=FAIL_MARKER,
                                expect_counter=(ARM_TRIES - i + 1, i - 1))
        results["phases"][f"post-fail-{i - 1}"] = post
        fails.append(rec)

    # --- boot 5: automatic fallback ------------------------------------------------
    rec5, ev5, post5 = do_boot(5, "fallback", V1, SLOT_A, "probe")
    results["phases"]["final"] = post5

    # --- acceptance ----------------------------------------------------------------
    fail_checks = [c for rec in fails for c in rec["checks"].values()]
    fallback_ok = (
        all(c["pass"] for c in rec5["checks"].values())
        and post5["states"].get(f"ingot_{V1}") == "good"
        and post5["states"].get(f"ingot_{V2}+0-{ARM_TRIES}") == "bad"
        and post5["default"] == f"ingot_{V1}"
    )
    results["acceptance"] = {
        # both slots carry versioned labels; v0.2.0 was built and its exact
        # slot artifact content is in the _empty slot B
        "build_and_labels": {
            "pass": (f"ingot_{V1}" in labels and f"ingot_{V2}" in labels
                     and written_sha == m2["artifacts"]["slot_erofs"]["sha256"]),
            "detail": f"labels={labels}",
        },
        # booting v0.2.0 to multi-user blesses it: counters reset, the entry
        # is good (no counter suffix)
        "bless_on_success": {
            "pass": all(c["pass"] for c in rec1["checks"].values()) and blessed,
            "detail": f"esp_after={post1['entries']} states={post1['states']}",
        },
        # the injected v0.2.0 failure falls back automatically: the three
        # failed boots exhaust the counter, the exhausted entry is bad and
        # not selected, v0.1.0 is preserved and boots
        "auto_fallback": {
            "pass": all(c["pass"] for c in fail_checks) and fallback_ok,
            "detail": (f"fail_counter_states="
                       f"{[r['selection_after']['entries'] for r in fails]} "
                       f"final={post5['entries']} default={post5['default']}"),
        },
        # boot selection state (default entry, counters, bless state) is
        # machine-readable at every phase
        "machine_readable_state": {
            "pass": all("default" in results["phases"][k]
                        and "entries" in results["phases"][k]
                        for k in ("deploy", "post-bless", "post-fail-1",
                                  "post-fail-2", "post-fail-3", "final")),
            "detail": "selection state recorded at every phase in ab-results.json",
        },
    }
    results["boots"] = boots
    results["slot_b_written_sha"] = written_sha
    results["pass"] = (all(v["pass"] for v in results["acceptance"].values())
                       and all(c["pass"] for c in
                               [c for r in boots for c in r["checks"].values()]))

    out = WORK / "ab-results.json"
    out.write_text(json.dumps(results, indent=2) + "\n")
    for name, a in results["acceptance"].items():
        print(f"  {'PASS' if a['pass'] else 'FAIL'}  {name}  ({a['detail']})")
    print(f"results: {out}")
    return 0 if results["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
