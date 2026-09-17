#!/bin/bash
# Ingot boot harness: boots the deployed disk in QEMU with KVM +
# pinned OVMF (snakeoil Secure Boot keys) and asserts the T1 boot
# invariants. Machine-readable results: exit code (0 = all pass) and
# dist/harness/results.json.
#
# Usage: harness/run.sh [--disk PATH]
set -euo pipefail

cd "$(dirname "$0")/.."
REPO=$(pwd)
DIST=$REPO/dist
WORK=$DIST/harness
DISK=${DISK:-$DIST/ingot_0.1.0.raw}
TIMEOUT=${TIMEOUT:-420}

mkdir -p "$WORK"

pins=$(python3 - <<EOF
import json
print(json.dumps(json.load(open("harness/pins.json"))))
EOF
)

pin_get() { python3 -c "import json,sys;print(json.loads(sys.argv[1])$1$2)" "$pins"; }

ovmf_code=$(pin_get "['ovmf']['code']" "['path']")
ovmf_code_sha=$(pin_get "['ovmf']['code']" "['sha256']")
ovmf_vars=$(pin_get "['ovmf']['vars']" "['path']")
ovmf_vars_sha=$(pin_get "['ovmf']['vars']" "['sha256']")

# --- pin verification (firmware blobs) -----------------------------------
for pair in "$ovmf_code:$ovmf_code_sha" "$ovmf_vars:$ovmf_vars_sha"; do
    path=${pair%%:*}; want=${pair##*:}
    [ -f "$path" ] || { echo "harness: missing firmware $path" >&2; exit 1; }
    got=$(sha256sum "$path" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "harness: firmware sha256 mismatch for $path" >&2; exit 1; }
done
echo "harness: firmware pins verified"

# --- fresh work copies ----------------------------------------------------
cp -f "$DISK" "$WORK/disk.img"
cp -f "$ovmf_vars" "$WORK/vars.fd"

console=$WORK/console.log
monitor=$WORK/monitor.sock
rm -f "$console" "$monitor"

echo "harness: booting (timeout ${TIMEOUT}s)..."
qemu-system-x86_64 \
    -machine q35,smm=on \
    -accel kvm \
    -cpu host \
    -m 4G \
    -drive if=pflash,format=raw,unit=0,readonly=on,file="$ovmf_code" \
    -drive if=pflash,format=raw,unit=1,file="$WORK/vars.fd" \
    -drive id=disk,if=none,format=raw,file="$WORK/disk.img" \
    -device virtio-blk-pci,drive=disk \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -serial file:"$console" \
    -monitor unix:"$monitor",server,nowait \
    -display none \
    -no-reboot &
qemu_pid=$!

cleanup() {
    if kill -0 "$qemu_pid" 2>/dev/null; then
        kill -TERM "$qemu_pid" 2>/dev/null || true
        wait "$qemu_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# --- wait for the probe frame ---------------------------------------------
found=0
deadline=$(( $(date +%s) + TIMEOUT ))
while [ "$(date +%s)" -lt "$deadline" ]; do
    if grep -q 'INGOT-HARNESS-PROBE-END' "$console" 2>/dev/null; then
        found=1
        break
    fi
    if ! kill -0 "$qemu_pid" 2>/dev/null; then
        break
    fi
    sleep 2
done

if [ "$found" = 1 ]; then
    # graceful shutdown via the monitor
    python3 - "$monitor" <<'EOF'
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
s.sendall(b"system_powerdown\n")
time.sleep(0.5)
s.close()
EOF
    ( sleep 60; kill -TERM "$qemu_pid" 2>/dev/null || true ) &
    watchdog=$!
    wait "$qemu_pid" 2>/dev/null || true
    kill "$watchdog" 2>/dev/null || true
fi

# --- assert ----------------------------------------------------------------
probe=$(python3 - "$console" <<'EOF'
import json, re, sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
begin = "=== INGOT-HARNESS-PROBE-BEGIN ==="
end = "=== INGOT-HARNESS-PROBE-END ==="
b = text.find(begin)
e = text.find(end, b + len(begin)) if b >= 0 else -1
if b < 0 or e <= b:
    print("{}")
else:
    # Console lines carry a kernel log prefix ("[  6.0364] sh[661]: ")
    # that the serial capture interleaves with the frame, and the
    # serial getty's interactive shell injects ANSI/DEC terminal
    # sequences into the stream; strip both, drop any non-JSON console
    # line that interleaved into the frame, then validate what remains.
    pre = re.compile(r"^\[\s*[\d.]+\]\s+\S+(?:\[\d+\])?:\s?")
    esc = re.compile(r"\x1b\[[0-9;?]*[\x20-\x2f]*[\x40-\x7e]"
                      r"|\x1b\][^\x1b\x07]*(?:\x1b\\|\x07)"
                      r"|\x1b[\x30-\x39\x40-\x7e]")
    out = []
    for ln in text[b + len(begin):e].split("\n"):
        ln = esc.sub("", ln)
        ln = pre.sub("", ln.rstrip("\r"), count=1)
        s = ln.strip()
        if s and s[0] in '"{}':
            out.append(ln)
    try:
        json.loads("\n".join(out))  # validate
        sys.stdout.write("\n".join(out))
    except json.JSONDecodeError:
        print("{}")
EOF
)

if python3 - "$WORK/results.json" "$probe" "$console" <<'EOF'
import json, sys
out_path, probe_raw, console_path = sys.argv[1], sys.argv[2], sys.argv[3]
p = json.loads(probe_raw) if probe_raw.strip() else {}

checks = {}

def add(name, ok, detail):
    checks[name] = {"pass": bool(ok), "detail": detail}

cmdline = p.get("cmdline", "")
usr = p.get("usr", {})
var = p.get("var", {})
etc = p.get("etc", {})
console = open(console_path, encoding="utf-8", errors="replace").read()

# 1. UEFI boot with Secure Boot enabled. The kernel's own boot-console
#    statement is authoritative; the guest-side efivar (when the guest
#    exposes /sys/firmware/efi/efivars) must agree.
console_sb = "Secure boot enabled" in console
efi = str(p.get("sysfs_efi", ""))
if "efivars" in efi:
    guest_sb = p.get("efivars_sb") == 1
elif efi:
    guest_sb = p.get("secure_boot") == 1
else:
    guest_sb = True  # guest exposes no efi sysfs; rely on the kernel
add("uefi_secure_boot",
    console_sb and guest_sb,
    f"console_sb={console_sb} sbs={p.get('secure_boot')} efivars_sb={p.get('efivars_sb')} sysfs_efi={efi!r}")

# 2. Boot from the UKI: its kernel command line is the running one
#    (state PARTUUID as root= + slot PARTUUID as usr=, as built by
#    mkosi)
add("boot_from_uki",
    "root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec" in cmdline
    and "rootfstype=btrfs" in cmdline
    and "usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc" in cmdline
    and "usrfstype=erofs" in cmdline,
    f"cmdline={cmdline!r}")

# 2b. / is a tmpfs (the minimal runtime root, CONTEXT.md)
add("root_tmpfs",
    p.get("root", {}).get("fstype") == "tmpfs",
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
    and "0066bfe5-47f1-52dc-9a16-1bb10191a1dc" in (usr.get("partuuid", "") + usr.get("source", "")),
    f"usr={usr!r}")

# 4b. /var is the btrfs state partition (mounted by the initramfs 99ingot
#     module before switch-root)
add("var_btrfs",
    var.get("fstype") == "btrfs",
    f"var={var!r}")

# 5. /etc present (bind over /var/lib/etc) with materialized fstab
add("etc_present",
    etc.get("is_mount") == 1 and etc.get("fstab_nonempty") == 1,
    f"etc={etc!r}")

# 6. multi-user reached, system running, journal without fatal errors,
#    no failed units
journal_err = p.get("journal_err", "")
failed = p.get("failed_units", "").strip()
add("multi_user_reached",
    p.get("multi_user") == "active",
    f"multi_user={p.get('multi_user')!r} running={p.get('system_running')!r}")
add("journal_clean",
    journal_err.strip() == "" and failed == "",
    f"failed_units={failed!r} journal_err={journal_err[:400]!r}")

# 7. brush is /bin/sh and system units started under it
add("brush_sh",
    p.get("sh") == "/usr/bin/brush",
    f"sh={p.get('sh')!r}")

results = {
    "pass": all(c["pass"] for c in checks.values()) and bool(checks),
    "checks": checks,
    "probe": p,
}
with open(out_path, "w") as f:
    json.dump(results, f, indent=2)
    f.write("\n")
for name, c in checks.items():
    print(f"  {'PASS' if c['pass'] else 'FAIL'}  {name}  ({c['detail']})")
print(f"results: {out_path}")
sys.exit(0 if results["pass"] else 1)
EOF
then rc=0; else rc=1; fi
echo "harness: $([ $rc -eq 0 ] && echo 'all assertions passed' || echo 'ASSERTION FAILURES')"
exit $rc
