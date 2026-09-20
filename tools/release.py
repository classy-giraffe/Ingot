#!/usr/bin/env python3
"""Ingot release pipeline orchestrator.

Produces the complete signed release asset set:
- ingot_<v>.root.erofs (extracted slot erofs payload, hard 2 GiB assertion)
- ingot_<v>.efi (signed UKI)
- ingot_<v>.iso (live ISO)
- SHA256SUMS (GNU sha256sum format of payload assets)
- SHA256SUMS.gpg (GPG signature)
- manifest.json (v1 schema release descriptor with build metadata per spec 20.4)
- manifest.json.gpg (GPG signature)
- Detached GPG signatures for all payload assets (*.gpg)

All signatures verify against the long-lived project public key.
"""

import argparse
import datetime
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAX_COMPRESSED_IMAGE_BYTES = 2 * 1024 * 1024 * 1024  # 2 GiB
WARN_IMAGE_BYTES = int(1.5 * 1024 * 1024 * 1024)  # 1.5 GiB
EROFS_SUPERBLOCK_OFFSET = 1024
EROFS_MAGIC = 0xE0F5E1E2


def sha256_file(path: Path) -> str:
    """Compute sha256 hex digest of a file."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1024 * 1024):
            h.update(chunk)
    return h.hexdigest()


def read_erofs_size(path: Path) -> tuple[int, int, int]:
    """Read erofs superblock to get block count, block size, and total bytes.
    Offset 1024 + 12: blkszbits (1 byte uint8, block size = 1 << blkszbits)
    Offset 1024 + 36: blocks (4 bytes uint32, filesystem block count)
    """
    with open(path, "rb") as f:
        f.seek(EROFS_SUPERBLOCK_OFFSET)
        header = f.read(128)

    if len(header) < 40:
        raise ValueError(f"File '{path}' is too small to contain an EROFS superblock")
    blkszbits = header[12]
    magic = struct.unpack_from("<I", header, 0)[0]
    if magic != EROFS_MAGIC:
        raise ValueError(
            f"File '{path}' does not contain an EROFS magic at offset {EROFS_SUPERBLOCK_OFFSET}"
        )

    blksize = 1 << blkszbits
    blocks = struct.unpack_from("<I", header, 36)[0]
    total_bytes = blocks * blksize
    return blocks, blksize, total_bytes


def extract_root_erofs(slot_raw: Path, output_path: Path) -> tuple[int, int, int]:
    """Extract trimmed slot erofs payload from slot.raw to output_path.

    Verifies superblock and streams exactly the filesystem blocks.
    """
    blocks, blksize, total_bytes = read_erofs_size(slot_raw)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(slot_raw, "rb") as src, open(output_path, "wb") as dst:
        remaining = total_bytes
        while remaining > 0:
            chunk_size = min(remaining, 1024 * 1024)
            data = src.read(chunk_size)
            if not data:
                raise ValueError(
                    f"Unexpected EOF reading {slot_raw} (wanted {total_bytes} bytes)"
                )
            dst.write(data)
            remaining -= len(data)

    return blocks, blksize, total_bytes


def assert_image_size(path: Path, max_bytes: int = MAX_COMPRESSED_IMAGE_BYTES) -> int:
    """Enforce hard 2 GiB assertion on compressed image size."""
    size = path.stat().st_size
    if size > max_bytes:
        raise ValueError(
            f"release: compressed image '{path.name}' size ({size} bytes) "
            f"exceeds 2 GiB ceiling ({max_bytes} bytes). Build fails."
        )
    if size > WARN_IMAGE_BYTES:
        print(
            f"release: WARNING - compressed image '{path.name}' size ({size} bytes) "
            f"exceeds 1.5 GiB (revisit firmware strategy per spec 20.4)",
            file=sys.stderr,
        )
    return size


def build_manifest(
    version: str,
    pins: dict,
    erofs_info: dict,
    kernel_nvr: str,
    assets: list[dict],
    timestamp: str | None = None,
) -> dict:
    """Assemble release manifest matching the v1 schema."""
    if timestamp is None:
        timestamp = datetime.datetime.now(datetime.timezone.utc).isoformat()

    return {
        "schema": 1,
        "version": version,
        "tag": f"v{version}",
        "image_version": version,
        "build": {
            "compose": pins.get("compose", {}).get("id", "unknown"),
            "component_pins": pins.get("rust", {}),
            "erofs": erofs_info,
            "kernel": kernel_nvr,
            "timestamp": timestamp,
        },
        "assets": assets,
    }


def sign_file(
    file_path: Path,
    secret_key_path: Path,
    output_path: Path | None = None,
) -> Path:
    """Sign file with GPG creating a detached signature."""
    if output_path is None:
        output_path = file_path.with_suffix(file_path.suffix + ".gpg")

    with tempfile.TemporaryDirectory() as td:
        home = Path(td)
        # Import secret key into temporary keyring
        subprocess.run(
            [
                "gpg",
                "--homedir",
                str(home),
                "--batch",
                "--quiet",
                "--import",
                str(secret_key_path),
            ],
            check=True,
            capture_output=True,
        )
        # Create detached signature
        subprocess.run(
            [
                "gpg",
                "--homedir",
                str(home),
                "--batch",
                "--yes",
                "--quiet",
                "--detach-sign",
                "--output",
                str(output_path),
                str(file_path),
            ],
            check=True,
            capture_output=True,
        )

    return output_path


def verify_file_signature(file_path: Path, sig_path: Path, pubkey_path: Path) -> bool:
    """Verify detached GPG signature against a public keyring."""
    with tempfile.TemporaryDirectory() as td:
        home = Path(td)
        # Import public key into temporary keyring
        res = subprocess.run(
            [
                "gpg",
                "--homedir",
                str(home),
                "--batch",
                "--quiet",
                "--import",
                str(pubkey_path),
            ],
            capture_output=True,
            check=False,
        )
        if res.returncode != 0:
            return False

        # Verify signature
        res = subprocess.run(
            [
                "gpg",
                "--homedir",
                str(home),
                "--batch",
                "--quiet",
                "--verify",
                str(sig_path),
                str(file_path),
            ],
            capture_output=True,
            check=False,
        )
        return res.returncode == 0


def build_release(
    version: str | None = None,
    dist_dir: Path | None = None,
    repo_root: Path = REPO_ROOT,
    secret_key_path: Path | None = None,
) -> dict:
    """Execute the release build pipeline.

    Assembles and signs all release assets, writes manifest.json,
    asserts size boundaries, and verifies all signatures against the project key.
    """
    if dist_dir is None:
        dist_dir = repo_root / "dist"
    pins_file = repo_root / "tools/pins.json"
    pins = json.loads(pins_file.read_text())
    if version is None:
        version = pins["image_version"]

    if secret_key_path is None:
        secret_key_path = repo_root / "tools/keys/project.sec"
    pubkey_path = repo_root / "tools/keys/project.pgp"

    if not secret_key_path.exists():
        raise FileNotFoundError(f"Project secret key not found at {secret_key_path}")
    if not pubkey_path.exists():
        raise FileNotFoundError(f"Project public key not found at {pubkey_path}")

    # 1. Ensure root.erofs payload
    root_erofs = dist_dir / f"ingot_{version}.root.erofs"
    slot_raw = dist_dir / f"ingot_{version}.slot.raw"

    if not root_erofs.exists():
        if not slot_raw.exists():
            raise FileNotFoundError(
                f"Neither {root_erofs} nor {slot_raw} found. Run tools/build.sh first."
            )
        print(f"release: extracting {root_erofs.name} from {slot_raw.name}...")
        blocks, blksize, total_bytes = extract_root_erofs(slot_raw, root_erofs)
    else:
        blocks, blksize, total_bytes = read_erofs_size(root_erofs)

    # Hard 2 GiB assertion on the compressed image
    image_size = assert_image_size(root_erofs)
    print(
        f"release: compressed image {root_erofs.name} verified ({image_size} bytes, {image_size / 1024 / 1024:.1f} MiB < 2 GiB)"
    )

    erofs_info = {
        "source_date_epoch": 1789516800,
        "compression": "zstd",
        "block_size": blksize,
        "blocks": blocks,
        "bytes": total_bytes,
    }

    # 2. Ensure UKI
    uki = dist_dir / f"ingot_{version}.efi"
    if not uki.exists():
        raise FileNotFoundError(f"UKI not found: {uki}. Run tools/build.sh first.")

    # 3. Ensure ISO
    iso = dist_dir / f"ingot_{version}.iso"
    if not iso.exists():
        build_iso_sh = repo_root / "tools/build-iso.sh"
        if build_iso_sh.exists():
            print("release: building live ISO...")
            subprocess.run(["sh", str(build_iso_sh)], check=True)
        if not iso.exists():
            raise FileNotFoundError(f"Live ISO not found: {iso}")

    # 4. Generate SHA256SUMS for payload assets
    payload_assets = [root_erofs, uki, iso]
    sums_lines = []
    sums_map = {}
    for p in payload_assets:
        digest = sha256_file(p)
        sums_map[p.name] = digest
        # Strict GNU sha256sum binary format required by systemd-sysupdate
        sums_lines.append(f"{digest} *{p.name}\n")

    sums_path = dist_dir / "SHA256SUMS"
    sums_path.write_text("".join(sums_lines))
    print(f"release: generated {sums_path.name} ({len(payload_assets)} assets)")

    # 5. Sign SHA256SUMS
    sums_sig = sign_file(sums_path, secret_key_path)
    print(f"release: signed {sums_sig.name}")

    # 6. Sign all payload assets
    asset_sigs = []
    for p in payload_assets:
        sig = sign_file(p, secret_key_path)
        asset_sigs.append(sig)
        print(f"release: signed {sig.name}")

    # 7. Collect all assets for manifest.json
    all_release_files = payload_assets + asset_sigs + [sums_path, sums_sig]
    manifest_assets = []
    roles = {
        f"ingot_{version}.root.erofs": "payload",
        f"ingot_{version}.efi": "uki",
        f"ingot_{version}.iso": "live",
    }
    for f in all_release_files:
        asset_entry = {
            "name": f.name,
            "sha256": sha256_file(f),
        }
        if f.name in roles:
            asset_entry["role"] = roles[f.name]
        manifest_assets.append(asset_entry)

    # 8. Kernel NVR
    kernel_nvr = "unknown"
    meta_json = dist_dir / f"build-metadata-{version}.json"
    if meta_json.exists():
        try:
            m = json.loads(meta_json.read_text())
            kernel_nvr = m.get("kernel", "unknown")
        except (json.JSONDecodeError, OSError):
            kernel_nvr = "unknown"

    # 9. Build and write manifest.json
    manifest = build_manifest(
        version=version,
        pins=pins,
        erofs_info=erofs_info,
        kernel_nvr=kernel_nvr,
        assets=manifest_assets,
    )
    manifest_path = dist_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(
        f"release: generated {manifest_path.name} (v1 schema, {len(manifest_assets)} assets)"
    )

    # 10. Sign manifest.json
    manifest_sig = sign_file(manifest_path, secret_key_path)
    print(f"release: signed {manifest_sig.name}")

    # 11. Verify all GPG signatures against public keyring
    all_sigs = asset_sigs + [sums_sig, manifest_sig]
    for sig in all_sigs:
        target_name = sig.name[:-4]  # strip .gpg
        target = dist_dir / target_name
        if not verify_file_signature(target, sig, pubkey_path):
            raise RuntimeError(
                f"Signature verification failed for {sig.name} against {pubkey_path}"
            )
        print(f"release: verified signature {sig.name} -> PASS")

    print("\nrelease: all assets produced and GPG signatures verified successfully")
    return {
        "version": version,
        "assets": [f.name for f in all_release_files + [manifest_path, manifest_sig]],
        "manifest": manifest,
    }


def main():
    parser = argparse.ArgumentParser(description="Ingot release pipeline builder")
    parser.add_argument(
        "--version", help="Release version (default: from tools/pins.json)"
    )
    parser.add_argument("--dist", type=Path, help="Dist directory (default: dist/)")
    parser.add_argument(
        "--key", type=Path, help="Project secret key (default: tools/keys/project.sec)"
    )
    args = parser.parse_args()

    try:
        build_release(
            version=args.version,
            dist_dir=args.dist,
            secret_key_path=args.key,
        )
    except (RuntimeError, ValueError, FileNotFoundError, OSError) as e:
        print(f"release error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
