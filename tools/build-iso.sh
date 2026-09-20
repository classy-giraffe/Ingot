#!/bin/sh
# tools/build-iso.sh - assemble the Ingot live ISO (spec 10, issue #8)
# from the dist/ artifacts produced by tools/build.sh.
#
# Output: dist/ingot_0.1.0.iso
#
# Layout (spec 10.3; the recommended tree, plus the hybrid boot
# machinery that makes it UEFI-bootable):
#   ISO9660 tree (volume label INGOTLIVE, Rock Ridge + Joliet):
#     EFI/BOOT/BOOTX64.EFI           the signed systemd-boot fallback
#                                    (carved from the installed ESP of
#                                    dist/ingot_0.1.0.raw: the same
#                                    snakeoil-signed binary the machine
#                                    gets on install)
#     EFI/Linux/ingot_0.1.0_live.efi the live UKI (the dist UKI with
#                                    the kernel command line swapped to
#                                    root=live; every other section is
#                                    byte-identical to the dist UKI,
#                                    verified below)
#     LiveOS/rootfs.erofs            the live payload: the slot erofs
#                                    size (the 8 GiB partition artifact
#                                    is padded with zeros)
#     esp/                           the installed machine's ESP tree
#                                    (carved from the installed ESP of
#                                    dist/ingot_0.1.0.raw): the live
#                                    install source (spec 10.2) deploys
#                                    it onto the new machine's ESP -
#                                    the systemd-boot fallback, the
#                                    installed UKI, the loader config
#     loader/loader.conf             the live loader config
#     ESP/efi.img                    the live ESP image (a FAT
#                                    filesystem carrying the same EFI/
#                                    tree and loader/): the El Torito
#                                    No-Emulation boot image and GPT
#                                    partition 1
#   GPT (isohybrid-gpt-basdat):
#     partition 1 (ESP): the live ESP - a FAT image carrying the same
#     EFI/ tree and loader/ (the UEFI firmware boots the ISO through
#     the El Torito No-Emulation boot image, which is the whole ISO:
#     it finds the GPT, finds the ESP partition, and loads
#     /EFI/BOOT/BOOTX64.EFI, whose /EFI/Linux autoentry boots the
#     live UKI)
#
# The ISO9660 volume label INGOTLIVE is what the live initramfs
# (image/files/usr/lib/dracut/modules.d/99ingot/prepare-live-root.sh)
# discovers the media by (by-label, with a block-device scan
# fallback).

set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
DIST=$REPO/dist
VERSION=0.1.0
UKI=$DIST/ingot_${VERSION}.efi
DISK=$DIST/ingot_${VERSION}.raw
SLOT_RAW=$DIST/ingot_${VERSION}.slot.raw
ISO=$DIST/ingot_${VERSION}.iso
KEYS=$REPO/tools/keys
ISO_LABEL=INGOTLIVE
command -v sbverify >/dev/null || { echo "build-iso: sbverify not found (sbsigntool)" >&2; exit 1; }
LIVE_CMDLINE="root=tmpfs ingot.live console=tty0 console=ttyS0 nowatchdog"

for f in "$UKI" "$DISK" "$SLOT_RAW" "$KEYS/snakeoil.key" "$KEYS/snakeoil.pem"; do
    [ -f "$f" ] || { echo "build-iso: missing input: $f (run tools/build.sh)" >&2; exit 1; }
done
command -v openssl >/dev/null || { echo "build-iso: openssl not found" >&2; exit 1; }
command -v xorriso >/dev/null || { echo "build-iso: xorriso not found" >&2; exit 1; }
command -v mkfs.vfat >/dev/null || { echo "build-iso: mkfs.vfat not found (dosfstools)" >&2; exit 1; }

WORK=$(mktemp -d /tmp/ingot-iso-XXXXXX)
trap 'rm -rf "$WORK"' EXIT

# --- 1. live UKI ------------------------------------------------------
# Byte-surgery on the dist UKI: replace the .cmdline section's content
# with the live command line (root=live instead of the PARTUUIDs) and
# re-sign it for Secure Boot. The dist UKI carries the installed kernel
# (snakeoil-signed), the initramfs, the CPU microcode, os-release, and
# the snakeoil Secure Boot signature; only the command line differs
# between the installed and the live boot.
#
# The dist UKI's signature is a systemd-sbsign-format Authenticode
# PKCS#7: a WIN_CERT2 appended at the end of the file that the PE
# security data directory points at (rva = file offset). The PKCS#7
# content is a SpcIndirectDataContent whose last 32 bytes are the
# UEFI-spec image hash; the messageDigest signed attribute is the
# SHA-256 of that content; and the RSA signature covers the canonical
# SET-of encoding of the signed attributes. OVMF (EDK2
# DxeImageVerificationLib + the OpenSSL AuthenticodeVerify) checks
# exactly this layout: the SpcIndirectDataContent's tail must equal the
# image hash and the PKCS#7 must verify against the key enrolled in DB.
# sbsign cannot cleanly re-sign this tightly packed UKI (the stale
# signature stays at the file's end and OVMF rejects it), so the
# re-sign is deterministic: swap the .cmdline bytes (the section's raw
# size is unchanged), truncate the old signature, zero the security
# data directory, recompute the image hash, clone the dist PKCS#7
# (replacing the image hash and the messageDigest, re-signing the
# attributes with the snakeoil key at the identical DER size), append
# the WIN_CERT2, and repoint the data directory at it.
python3 - "$UKI" "$WORK/live.efi" "$LIVE_CMDLINE" "$KEYS/snakeoil.key" <<'EOF'
import hashlib, struct, subprocess, sys

src, dst, cmdline, key = sys.argv[1:5]
b = bytearray(open(src, 'rb').read())
pe = struct.unpack_from('<I', b, 0x3c)[0]
nsec = struct.unpack_from('<H', b, pe + 6)[0]
opt = struct.unpack_from('<H', b, pe + 20)[0]
sec = pe + 24 + opt
o = pe + 24
new = cmdline.encode() + b'\x00'
last_end = 0
for i in range(nsec):
    oo = sec + i * 40
    rs, ro = struct.unpack_from('<II', b, oo + 16)
    if rs:
        last_end = max(last_end, ro + rs)
cmdline_swapped = False
for i in range(nsec):
    oo = sec + i * 40
    if b[oo:oo + 8].split(b'\0')[0].decode() != '.cmdline':
        continue
    vs, va, rs, ro = struct.unpack_from('<IIII', b, oo + 8)
    if len(new) > rs:
        sys.exit(f'live cmdline ({len(new)} bytes) exceeds the .cmdline '
                 f'section raw size ({rs} bytes)')
    b[ro:ro + len(new)] = new
    b[ro + len(new):ro + rs] = b'\x00' * (rs - len(new))
    struct.pack_into('<I', b, oo + 8, len(cmdline.encode()))  # vs = text
    cmdline_swapped = True
    break
if not cmdline_swapped:
    sys.exit('no .cmdline section in the dist UKI')

# UEFI image hash (EDK2 DxeImageVerificationLib HashPeImage): the
# headers up to the CheckSum field, from after the CheckSum to the
# security data directory entry, from after the entry to the end of
# the headers, the section raw data in file order, and the bytes
# between the last section and the appended signature.
def uefi_image_hash(buf, cert_size):
    soh = struct.unpack_from('<I', buf, o + 60)[0]
    h = hashlib.sha256()
    h.update(buf[0:o + 64])
    h.update(buf[o + 68:o + 144])
    h.update(buf[o + 152:soh])
    total = soh
    raws = []
    for i in range(nsec):
        oo = sec + i * 40
        rs, ro = struct.unpack_from('<II', buf, oo + 16)
        if rs:
            raws.append((ro, rs))
    for ro, rs in sorted(raws):
        h.update(buf[ro:ro + rs])
        total += rs
    if len(buf) > total:
        h.update(buf[total:len(buf) - cert_size])
    return h.digest()

# The security data directory (entry 4, PE32+ layout) points at the
# old signature.
dd = o + 112 + 4 * 8
old_rva, old_size = struct.unpack_from('<II', b, dd)
if old_rva != last_end or not old_size:
    sys.exit(f'unexpected security data directory '
             f'(rva={old_rva:#x} size={old_size:#x}, '
             f'last section ends at {last_end:#x})')

# Clone the dist PKCS#7 (WIN_CERT2 minus its 8-byte header) - the
# clone source is the old signature region, so read it before
# truncating.
p7 = bytearray(b[old_rva:old_rva + old_size][8:])

# Truncate the old signature (it covers the original .cmdline) and
# zero the security data directory.
del b[last_end:]
struct.pack_into('<II', b, dd, 0, 0)
imghash = uefi_image_hash(b, 0)

def rlen(buf, i):
    l = buf[i + 1]
    if l & 0x80:
        n = l & 0x7f
        return int.from_bytes(buf[i + 2:i + 2 + n], 'big'), 2 + n
    return l, 2

# The SpcIndirectDataContent: the PKCS#7 content, after the content
# type OID 1.3.6.1.4.1.311.2.1.4 (spcIndirectDataObject; the first
# occurrence is the contentInfo, the second is a signed attribute).
cid = p7.find(b'\x06\x0a\x2b\x06\x01\x04\x01\x82\x37\x02\x01\x04')
if cid < 0 or p7[cid + 12] != 0xa0 or p7[cid + 14] != 0x30:
    sys.exit('dist PKCS#7 lacks the spcIndirectDataObject content')
idc_off = cid + 14
idc_len, idc_hdr = rlen(p7, idc_off)
idc_inner_off = idc_off + idc_hdr
p7[idc_inner_off + idc_len - 32:idc_inner_off + idc_len] = imghash

# The messageDigest signed attribute (pkcs-9 messageDigest,
# 1.2.840.113549.1.9.4 - distinct from the contentType attribute,
# 1.9.3): SHA-256 of the (now patched) SpcIndirectDataContent.
md = p7.find(b'\x06\x09' + bytes.fromhex('2a864886f70d010904'))
if md < 0 or p7[md + 11] != 0x31:
    sys.exit('dist PKCS#7 lacks the messageDigest signed attribute')
set_len, set_hdr = rlen(p7, md + 11)
ov = md + 11 + set_hdr
if p7[ov] != 0x04 or p7[ov + 1] != 0x20:
    sys.exit('unexpected messageDigest attribute value encoding')
p7[ov + 2:ov + 34] = hashlib.sha256(
    p7[idc_inner_off:idc_inner_off + idc_len]).digest()

# The signed attributes ([0] node before the encrAlg), re-signed in
# their canonical form (the same attribute bytes in a SET-of
# wrapper, OpenSSL's canonical encoding) with the snakeoil key.
rsa = p7.rfind(b'\x06\x09' + bytes.fromhex('2a864886f70d010101'))
j = rsa
while p7[j] != 0x30:
    j -= 1
el, eh = rlen(p7, j)
sig_off = j + eh + el
if p7[sig_off] != 0x04:
    sys.exit('unexpected PKCS#7 signature encoding')
sig_len, sig_hdr = rlen(p7, sig_off)
if sig_len != 256:
    sys.exit(f'unexpected PKCS#7 signature size {sig_len}')
k = sig_off
while True:
    k -= 1
    if p7[k] == 0xa0:
        al, ah = rlen(p7, k)
        if k + ah + al == j:
            break
sattrs = p7[k + ah:k + ah + al]

# The canonical form OpenSSL's PKCS7_verify hashes: a DER SET-of
# over the raw attribute bytes (definite, shortest-length form -
# the attributes exceed 127 bytes, so the length is long-form).
def der_len(n):
    if n <= 0x7f:
        return bytes([n])
    m = n.to_bytes((n.bit_length() + 7) // 8, 'big')
    return bytes([0x80 | len(m)]) + m

canonical = b'\x31' + der_len(len(sattrs)) + sattrs
# DigestInfo with the NIST legacy SHA-256 OID, as this toolchain's
# OpenSSL emits for RSA PKCS#1 v1.5 signatures (OVMF's OpenSSL-based
# verifier accepts both this and the standard OID).
digest_info = (bytes.fromhex('3031300d060960864801650304020105000420')
               + hashlib.sha256(canonical).digest())
block = (b'\x00\x01' + b'\xff' * (256 - 3 - len(digest_info))
         + b'\x00' + digest_info)
subprocess.run('openssl rsautl -sign -raw -inkey %s -out %s' % (key, dst + '.sig'),
               shell=True, check=True, input=block)
p7[sig_off + sig_hdr:sig_off + sig_hdr + 256] = open(dst + '.sig', 'rb').read()

# Reassemble: the WIN_CERT2 (dwLength, revision 0x0200, type
# at the end of the file, the security data directory repointed at it.
win = struct.pack('<IHH', len(p7) + 8, 0x0200, 0x0002) + bytes(p7)
b += win
struct.pack_into('<II', b, dd, len(b) - len(win), len(win))
open(dst, 'wb').write(bytes(b))
print(f'live UKI: .cmdline swapped to {cmdline!r}, Secure Boot '
      f'signature re-signed (image hash {imghash.hex()[:16]}..., '
      f'WIN_CERT2 at {len(b) - len(win):#x})')
EOF

# Verify: every section of the live UKI except .cmdline must be
# byte-identical to the dist UKI (the kernel, initramfs, microcode,
# os-release, stub, and SBAT are the installed release's), the
# .cmdline must carry the live command line, and the signature must
# verify (the Authenticode layout OVMF checks plus the pinned cert).
python3 - "$UKI" "$WORK/live.efi" "$LIVE_CMDLINE" <<'EOF'
import struct, sys
def sections(path):
    b = open(path, 'rb').read()
    pe = struct.unpack_from('<I', b, 0x3c)[0]
    nsec = struct.unpack_from('<H', b, pe + 6)[0]
    opt = struct.unpack_from('<H', b, pe + 20)[0]
    sec = pe + 24 + opt
    out = {}
    for i in range(nsec):
        o = sec + i * 40
        name = b[o:o + 8].split(b'\0')[0].decode()
        vs, va, rs, ro = struct.unpack_from('<IIII', b, o + 8)
        out[name] = b[ro:ro + rs]
    return out
dist, live = sections(sys.argv[1]), sections(sys.argv[2])
if set(dist) != set(live):
    sys.exit(f'section sets differ: {set(dist) ^ set(live)}')
for name in sorted(dist):
    if name == '.cmdline':
        text = live[name].rstrip(b'\x00').decode()
        if text != sys.argv[3]:
            sys.exit(f'live UKI .cmdline is {text!r}, expected {sys.argv[3]!r}')
    elif dist[name] != live[name]:
        sys.exit(f'section {name} differs from the dist UKI')
print(f'live UKI: all sections identical to the dist UKI except .cmdline ({sys.argv[3]!r})')
EOF
sbverify --cert "$KEYS/snakeoil.pem" "$WORK/live.efi" >/dev/null \
    || { echo "build-iso: live UKI signature failed verification" >&2; exit 1; }

# --- 2. the live ESP ---------------------------------------------------
# The signed fallback loader comes from the installed ESP (the same
# snakeoil-signed systemd-boot binary the installed machine carries).
python3 - "$DISK" "$WORK/esp-installed.fat" <<EOF
import sys
sys.path.insert(0, "$REPO/harness")
from esp import carve_esp
carve_esp(sys.argv[1], sys.argv[2])
EOF
mtools() { MTOOLS_SKIP_CHECK=1 "$@"; }
mtools mcopy -b -i "$WORK/esp-installed.fat" "::/EFI/BOOT/BOOTX64.EFI" "$WORK/BOOTX64.EFI"

# The live ESP: a FAT image with the fallback loader, the live UKI,
# and the loader config. Size: the live UKI (~225 MiB) plus headroom.
truncate -s 256M "$WORK/esp.fat"
mkfs.vfat -n INGOTESP -F 16 "$WORK/esp.fat" >/dev/null
mtools mmd -i "$WORK/esp.fat" ::/EFI
mtools mmd -i "$WORK/esp.fat" ::/EFI/BOOT
mtools mmd -i "$WORK/esp.fat" ::/EFI/Linux
mtools mmd -i "$WORK/esp.fat" ::/loader
mtools mcopy -i "$WORK/esp.fat" "$WORK/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
mtools mcopy -i "$WORK/esp.fat" "$WORK/live.efi" ::/EFI/Linux/ingot_${VERSION}_live.efi
printf 'timeout 5\nconsole-mode max\n' > "$WORK/loader.conf"
mtools mcopy -i "$WORK/esp.fat" "$WORK/loader.conf" ::/loader/loader.conf

# --- 3. the live payload -----------------------------------------------
# Trim the slot erofs to its actual filesystem size (the superblock's
# block count; the 8 GiB partition artifact is zero-padded beyond
# it). The trimmed file is what the ISO carries and what the live
# initramfs mounts at /usr.
# the filesystem size from the superblock (dump.erofs reads the block
# count; the block size is 4 KiB on x86_64)
blocks=$(dump.erofs "$SLOT_RAW" | awk '/^Filesystem blocks:/ {print $3}')
[ -n "$blocks" ] || { echo "build-iso: cannot determine the slot erofs size" >&2; exit 1; }
erofs_bytes=$((blocks * 4096))
dd if="$SLOT_RAW" of="$WORK/rootfs.erofs" bs=4096 count="$blocks" status=none
# the padding beyond the filesystem must be zeros (sanity: the block
# count must not understate the data)
python3 - "$SLOT_RAW" "$erofs_bytes" <<'EOF'
import sys
path, end = sys.argv[1], int(sys.argv[2])
f = open(path, 'rb')
f.seek(end)
while chunk := f.read(1 << 20):
    if any(chunk):
        sys.exit(1)
EOF
if [ $? -ne 0 ]; then
    echo "build-iso: the slot raw is not zero-padded beyond the erofs" >&2
    exit 1
fi
echo "live payload: $erofs_bytes bytes ($((erofs_bytes / 1024 / 1024)) MiB of the $(stat -c %s "$SLOT_RAW" | awk '{printf "%.0f", $1/1073741824}') GiB partition artifact)"

# --- 4. the installed ESP tree (the live install source) ----------------
# The installer's live source mode (spec 10.2) deploys the media's esp/
# tree onto the new machine's ESP. The source of truth is the installed
# machine's ESP (carved as esp-installed.fat in section 2): extract its
# whole tree and verify it - the systemd-boot fallback (which must
# byte-match the loader carved for the live ESP), the installed UKI,
# and the loader config.
mkdir -p "$WORK/esp-tree"
mtools mcopy -b -s -i "$WORK/esp-installed.fat" "::/*" "$WORK/esp-tree/"
for f in "EFI/BOOT/BOOTX64.EFI" "EFI/Linux/ingot_${VERSION}.efi" "loader/loader.conf"; do
    [ -f "$WORK/esp-tree/$f" ] \
        || { echo "build-iso: installed ESP tree missing $f" >&2; exit 1; }
done
sbverify --cert "$KEYS/snakeoil.pem" "$WORK/esp-tree/EFI/BOOT/BOOTX64.EFI" >/dev/null \
    || { echo "build-iso: the installed ESP fallback is not snakeoil-signed" >&2; exit 1; }
sbverify --cert "$KEYS/snakeoil.pem" "$WORK/esp-tree/EFI/Linux/ingot_${VERSION}.efi" >/dev/null \
    || { echo "build-iso: the installed UKI is not snakeoil-signed" >&2; exit 1; }
cmp -s "$WORK/esp-tree/EFI/BOOT/BOOTX64.EFI" "$WORK/BOOTX64.EFI" \
    || { echo "build-iso: installed and live ESP fallbacks differ" >&2; exit 1; }

# --- 5. the ISO tree (spec 10.3) ----------------------------------------

mkdir -p "$WORK/iso/EFI/BOOT" "$WORK/iso/EFI/Linux" "$WORK/iso/LiveOS" "$WORK/iso/loader" "$WORK/iso/ESP"
cp "$WORK/BOOTX64.EFI" "$WORK/iso/EFI/BOOT/BOOTX64.EFI"
cp "$WORK/live.efi" "$WORK/iso/EFI/Linux/ingot_${VERSION}_live.efi"
cp "$WORK/live.efi" "$WORK/iso/EFI/Linux/live.efi"
cp "$WORK/rootfs.erofs" "$WORK/iso/LiveOS/rootfs.erofs"
cp "$WORK/loader.conf" "$WORK/iso/loader/loader.conf"
# the installed ESP tree (section 4): the live install source's
# esp/ (the installer deploys it onto the new machine's ESP)
cp -a "$WORK/esp-tree" "$WORK/iso/esp"
# the live ESP image inside the tree: the El Torito No-Emulation boot
# image (and GPT partition 1) is this file, referenced by -e below
cp "$WORK/esp.fat" "$WORK/iso/ESP/efi.img"

# --- 6. ISO assembly -----------------------------------------------------
# The hybrid boot machinery (UEFI firmware's path):
#   -e "ESP/efi.img" -no-emul-boot: the El Torito No-Emulation boot
#   image is the live ESP (platform 0xef).
#   efi_boot_part=--efi-boot-image: the GPT exposes that boot image as
#   the EFI System Partition, and the ISO9660 data region (the "basdat"
#   of -isohybrid-gpt-basdat) becomes data partitions around it. So the
#   UEFI firmware boots the ISO by finding the ESP partition and
#   loading /EFI/BOOT/BOOTX64.EFI from it.
# This xorriso (1.5.6, Ubuntu noble) has no -isohybrid-gpt-basdat
# shortcut (it is silently ignored in mkisofs mode, leaving no GPT);
# the -boot_image bootspec after -- is the native mechanism.
xorriso -as mkisofs \
    -iso-level 2 \
    -full-iso9660-filenames \
    -R -J \
    -V "$ISO_LABEL" \
    -o "$ISO" \
    -e "ESP/efi.img" -no-emul-boot \
    "$WORK/iso/" \
    -- \
    -boot_image any efi_boot_part=--efi-boot-image

# --- 7. ISO verification -------------------------------------------------
# The GPT must carry an ESP-type partition (the firmware's boot path:
# the El Torito No-Emulation boot image, which the firmware finds via
# the GPT and loads /EFI/BOOT/BOOTX64.EFI from), and the ISO9660
# volume must carry the spec tree under the label the live initramfs
# discovers the media by. The tree check (xorriso) asserts the live
# payload, the live boot tree, and the installed ESP tree the live
# install deploys.
python3 - "$ISO" "$ISO_LABEL" <<EOF
import sys
sys.path.insert(0, "$REPO/harness")
import gpt
disk, label = sys.argv[1], sys.argv[2]
g = gpt.read_gpt(disk)
types = [(p.type_uuid, p.label) for p in g.entries]
ESP_TYPE = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
esp = [p for p in g.entries if p.type_uuid == ESP_TYPE]
if not esp:
    sys.exit(f'ISO GPT has no ESP partition: {types}')
# PVD volume label: in the isohybrid layout the PVD sits at the basdat
# offset (not sector 16); scan the first MiB for the primary volume
# descriptor signature.
with open(disk, "rb") as f:
    head = f.read(1024 * 1024)
i = head.find(b"CD001")
if i < 0 or head[i - 1] != 1:
    sys.exit("ISO9660 primary volume descriptor not found in the first MiB")
vol = head[i + 39:i + 56].decode("latin-1").strip()
if vol != label:
    sys.exit(f"ISO volume label {vol!r} != {label!r}")
print(f"ISO GPT: {len(types)} partition(s), ESP at LBA {esp[0].first_lba} "
      f"({esp[0].size_bytes // 1024 // 1024} MiB); PVD label {vol!r}")
EOF
# The ISO9660 tree must carry the spec layout: the live payload, the
# live boot tree, and the installed ESP tree the live install deploys.
iso_files=$(xorriso -indev "$ISO" -find / 2>/dev/null | tr -d "'")
for f in "LiveOS/rootfs.erofs" \
    "EFI/BOOT/BOOTX64.EFI" "EFI/Linux/ingot_${VERSION}_live.efi" \
    "loader/loader.conf" "ESP/efi.img" \
    "esp/EFI/BOOT/BOOTX64.EFI" "esp/EFI/Linux/ingot_${VERSION}.efi" \
    "esp/loader/loader.conf"; do
    printf '%s\n' "$iso_files" | grep -qxF "/$f" \
        || { echo "build-iso: ISO tree missing /$f" >&2; exit 1; }
done
echo "ISO tree: live payload + live boot tree + installed ESP tree present"
