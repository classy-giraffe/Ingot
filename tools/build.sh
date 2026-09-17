#!/bin/bash
# Ingot build orchestrator.
#
# Usage:
#   tools/build.sh [--local]
#
# Steps:
#   pins     tools/check-pins.py (host tools, firmware, keys, compose
#            pin consistency between tools/pins.json and the mkosi conf)
#   image    mkosi build of the main image: the whole system as a GPT
#            disk (signed UKI + fallback on the ESP, the slot erofs,
#            the btrfs state partitions). Split artifacts: the erofs
#            slot (SplitName=slot), the standalone UKI, kernel, initrd.
#   verify   UKI PE sections + PARTUUIDs in the .cmdline; disk layout
#            (fixed-UUID repart definitions); build metadata
#            (dist/build-metadata.json, spec 20.4).
#
# --local: build against the archived compose (--local-mirror) instead
#          of the network. Requires the archive (tools/archive-compose.sh).
set -euo pipefail

cd "$(dirname "$0")/.."
REPO=$(pwd)
DIST=$REPO/dist
IMAGE_VERSION=$(python3 -c 'import json;print(json.load(open("tools/pins.json"))["image_version"])')
ARCHIVE_DIR=$(python3 -c 'import json;print(json.load(open("tools/pins.json"))["compose"]["archive_dir"])')
ARCHIVE_TREE="$ARCHIVE_DIR/compose/Everything/x86_64/os"
DISK=$DIST/ingot_${IMAGE_VERSION}.raw
SLOT=$DIST/ingot_${IMAGE_VERSION}.slot.raw
UKI=$DIST/ingot_${IMAGE_VERSION}.efi
# hyphen name: the underscore name is mkosi's own split initrd artifact
INITRD=$DIST/ingot-${IMAGE_VERSION}.initrd

local_mirror=0
for arg in "$@"; do
    case $arg in
        --local) local_mirror=1 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

# --- pins ------------------------------------------------------------------
python3 tools/check-pins.py

# --- mkosi main image --------------------------------------------------------
mkdir -p "$DIST"
# mkosi validates the --initrd path up front, and a non-empty Initrds=
# (any --initrd) suppresses the stock default-initrd subimage. The real
# initramfs is produced in-image by the postinst chroot and staged into
# the artifacts directory (io.mkosi.initrd/), which mkosi embeds in the
# UKI's .initrd section in addition to the file passed here. So this
# file only needs to exist and be a valid cpio: generate an empty newc
# archive (the kernel unpacks chained cpio archives in sequence; the
# empty one contributes nothing).
mkosi_args=(-f -C image build --initrd "$INITRD")
rm -f "$INITRD"
printf '' | cpio -0 -o -H newc --quiet > "$INITRD"
if [ "$local_mirror" = 1 ]; then
    [ -d "$ARCHIVE_TREE" ] || { echo "archive missing: $ARCHIVE_TREE (run tools/archive-compose.sh)" >&2; exit 1; }
    [ -f "$ARCHIVE_DIR/compose/manifest.json" ] \
        || { echo "archive manifest missing (incomplete mirror)" >&2; exit 1; }
    mkosi_args+=("--local-mirror" "file://$ARCHIVE_TREE")
fi
mkosi "${mkosi_args[@]}"

# --- artifact verification ----------------------------------------------------
[ -f "$DISK" ] || { echo "disk image missing: $DISK" >&2; exit 1; }
[ -f "$SLOT" ] || { echo "slot erofs split artifact missing: $SLOT" >&2; exit 1; }
[ -f "$UKI" ] || { echo "UKI split artifact missing: $UKI" >&2; exit 1; }

# UKI verification (acceptance: the UKI embeds the kernel, initramfs,
# CPU microcode, and the fixed-UUID slot/state PARTUUIDs, and carries
# the snakeoil Secure Boot signature). The signature is stored in the
# PE security data directory (entry 4) by systemd-sbsign, not in a
# .rsrc section: check that the entry is present and that the
# certificate it carries is the pinned snakeoil certificate.
sechdr=$(objdump -h "$UKI")
for s in linux initrd ucode cmdline osrel; do
    echo "$sechdr" | grep -qw "$s" \
        || { echo "UKI is missing section .$s" >&2; exit 1; }
done
python3 - "$UKI" "$REPO/tools/keys/snakeoil.pem" <<'EOF'
import base64, struct, sys
uki, pem = sys.argv[1], sys.argv[2]
b = open(uki, 'rb').read()
pe = struct.unpack_from('<I', b, 0x3c)[0]
assert b[pe:pe + 4] == b'PE\0\0', "not a PE image"
magic = struct.unpack_from('<H', b, pe + 24)[0]
dd = pe + 24 + (112 if magic == 0x20b else 92)  # data directories
rva, size = struct.unpack_from('<II', b, dd + 4 * 8)  # entry 4: security
assert rva and size, "PE security directory empty: UKI is unsigned"
der = base64.b64decode(''.join(
    l for l in open(pem) if not l.startswith('-----')))

# Locate the signature: standard PEs keep it inside a section (map the
# RVA through the section table); systemd-sbsign appends it at the end
# of the file and records the file offset in the security directory.
# Accept the certificate in either region.
offs = []
nsec = struct.unpack_from('<H', b, pe + 6)[0]
opt = struct.unpack_from('<H', b, pe + 20)[0]
sec = pe + 24 + opt
for i in range(nsec):
    o = sec + i * 40
    vs, va, rs, ro = struct.unpack_from('<IIII', b, o + 8)
    if va <= rva < va + max(vs, 1):
        offs.append(ro + (rva - va))
        break
if rva + size <= len(b):
    offs.append(rva)

assert any(der in b[o:o + size] for o in offs), \
    "signature does not contain the pinned snakeoil certificate"
print(f"UKI signature: security directory present (rva=0x{rva:x} size=0x{size:x}), snakeoil certificate verified")
EOF
# .cmdline carries the fixed-UUID slot/state PARTUUIDs; read the
# section bytes directly (objdump -s wraps at 16 bytes per line, so a
# plain grep for the 36-char PARTUUIDs would miss)
python3 - "$UKI" <<'EOF'
import struct, sys
b = open(sys.argv[1], 'rb').read()
pe = struct.unpack_from('<I', b, 0x3c)[0]
nsec = struct.unpack_from('<H', b, pe + 6)[0]
opt = struct.unpack_from('<H', b, pe + 20)[0]
sec = pe + 24 + opt
cmd = None
for i in range(nsec):
    o = sec + i * 40
    name = b[o:o + 8].split(b'\0')[0].decode()
    if name == '.cmdline':
        rs, ro = struct.unpack_from('<II', b, o + 16)
        cmd = b[ro:ro + rs].rstrip(b'\0').decode()
        break
assert cmd, "UKI .cmdline section missing"
for want in ("usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
             "root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec"):
    assert want in cmd, f"UKI .cmdline lacks {want}"
print(f"UKI .cmdline: {cmd}")
EOF
echo "build: UKI verified ($UKI: .linux/.initrd/.ucode/.cmdline/.osrel + signature, slot PARTUUIDs)"

# disk layout sanity (acceptance: the fixed-UUID repart definitions
# produce the full layout - ESP, active slot with versioned label,
# _empty slot, /var and /home on btrfs)
layout=$(sfdisk -d "$DISK" 2>/dev/null || true)
for uuid in \
    f357e520-642b-5b9b-abc7-9e5969de5691 \
    0066bfe5-47f1-52dc-9a16-1bb10191a1dc \
    59f1269b-a18e-558b-a225-8394c1d1bc0f \
    501347aa-775a-5736-8da3-2a9977c820ec \
    9e772d30-44c4-5784-8be1-0832e3c1c4db; do
    echo "$layout" | grep -qi "$uuid" \
        || { echo "disk is missing the partition UUID $uuid" >&2; exit 1; }
done
echo "build: disk layout verified ($DISK: esp/ingot_${IMAGE_VERSION}/_empty/var/home, fixed PARTUUIDs)"

# --- build metadata (spec 20.4: record the build parameters) --------------------
kv=$(python3 - "$DIST/ingot_${IMAGE_VERSION}.manifest" <<'EOF'
import json, sys
m = json.load(open(sys.argv[1]))
for p in m.get("packages", []):
    if p.get("name") == "kernel-core":
        print(p["version"] + "-" + str(p.get("release", "")))
        break
EOF
)
erofs_sha=$(sha256sum "$SLOT" | cut -d' ' -f1)
python3 - "$erofs_sha" "$kv" <<'EOF'
import json, sys, pathlib
sha, kv = sys.argv[1], sys.argv[2]
pins = json.load(open("tools/pins.json"))
meta = {
    "image_id": "ingot",
    "image_version": pins["image_version"],
    "compose": pins["compose"]["id"],
    "kernel": kv,
    "rust": pins["rust"],
    "artifacts": {
        "disk": f"ingot_{pins['image_version']}.raw",
        "slot_erofs": {
            "artifact": f"ingot_{pins['image_version']}.slot.raw",
            "sha256": sha,
            "parameters": {
                "source_date_epoch": 1789516800,
                "compression": "zstd",
                "mechanism": "systemd-repart Format=erofs Compression=zstd (mkosi disk output, CopyFiles=/usr:/)",
            },
        },
    },
}
pathlib.Path("dist/build-metadata.json").write_text(json.dumps(meta, indent=2) + "\n")
print("metadata: dist/build-metadata.json")
EOF

echo "build: done"

# hand ownership back to the invoking user (the build may have run as
# root via sudo, which the mkosi sandbox requires in restricted
# environments); the harness and later steps run unprivileged
if [ -n "${SUDO_USER:-}" ] && [ "$(id -u)" = 0 ]; then
    chown -R "$SUDO_USER:$SUDO_UID" "$DIST"
fi
