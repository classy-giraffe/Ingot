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
import contextlib
import datetime
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAX_COMPRESSED_IMAGE_BYTES = 2 * 1024 * 1024 * 1024  # 2 GiB
EROFS_SUPERBLOCK_OFFSET = 1024
EROFS_MAGIC = 0xE0F5E1E2


@dataclass(frozen=True)
class ErofsInfo:
    """EROFS filesystem sizing parameters."""

    blocks: int
    blksize: int
    total_bytes: int

    def __iter__(self):
        yield self.blocks
        yield self.blksize
        yield self.total_bytes


def canonical_asset_names(version: str) -> list[str]:
    """Canonical list of assets for an Ingot release."""
    payload = [
        f"ingot_{version}.root.erofs",
        f"ingot_{version}.efi",
        f"ingot_{version}.iso",
    ]
    sigs = [f"{name}.gpg" for name in payload]
    metadata = ["SHA256SUMS", "SHA256SUMS.gpg", "manifest.json", "manifest.json.gpg"]
    return payload + sigs + metadata


def sha256_file(path: Path) -> str:
    """Compute sha256 hex digest of a file."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1024 * 1024):
            h.update(chunk)
    return h.hexdigest()


def read_erofs_size(path: Path) -> ErofsInfo:
    """Read erofs superblock to get block count, block size, and total bytes."""
    with open(path, "rb") as f:
        f.seek(EROFS_SUPERBLOCK_OFFSET)
        header = f.read(128)

    if len(header) < 40:
        raise ValueError(f"File '{path}' is too small to contain an EROFS superblock")

    magic = struct.unpack_from("<I", header, 0)[0]
    if magic != EROFS_MAGIC:
        raise ValueError(
            f"File '{path}' does not contain an EROFS magic at offset {EROFS_SUPERBLOCK_OFFSET}"
        )

    blkszbits = header[12]
    blksize = 1 << blkszbits
    blocks = struct.unpack_from("<I", header, 36)[0]
    total_bytes = blocks * blksize
    return ErofsInfo(blocks=blocks, blksize=blksize, total_bytes=total_bytes)


def extract_root_erofs(slot_raw: Path, output_path: Path) -> ErofsInfo:
    """Extract trimmed slot erofs payload from slot.raw to output_path."""
    info = read_erofs_size(slot_raw)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(slot_raw, "rb") as src, open(output_path, "wb") as dst:
        remaining = info.total_bytes
        while remaining > 0:
            chunk_size = min(remaining, 1024 * 1024)
            data = src.read(chunk_size)
            if not data:
                raise ValueError(
                    f"Unexpected EOF reading {slot_raw} (wanted {info.total_bytes} bytes)"
                )
            dst.write(data)
            remaining -= len(data)

    return info


def assert_image_size(path: Path, max_bytes: int = MAX_COMPRESSED_IMAGE_BYTES) -> int:
    """Enforce hard 2 GiB assertion on compressed image size."""
    size = path.stat().st_size
    if size > max_bytes:
        raise ValueError(
            f"release: compressed image '{path.name}' size ({size} bytes) "
            f"exceeds 2 GiB ceiling ({max_bytes} bytes). Build fails."
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


def load_build_metadata(meta_path: Path) -> dict:
    """Load build metadata from dist/build-metadata-<version>.json."""
    if not meta_path.exists():
        return {}
    try:
        data = json.loads(meta_path.read_text())
    except (json.JSONDecodeError, OSError):
        return {}

    slot_erofs = data.get("artifacts", {}).get("slot_erofs", {})
    params = slot_erofs.get("parameters", {})
    return {
        "kernel": data.get("kernel", "unknown"),
        "compose": data.get("compose"),
        "erofs_parameters": params,
    }


@contextlib.contextmanager
def gpg_temp_home(key_to_import: Path | None = None):
    """Context manager setting up an isolated temporary GPG home directory."""
    with tempfile.TemporaryDirectory() as td:
        home = Path(td)
        if key_to_import is not None and key_to_import.exists():
            subprocess.run(
                [
                    "gpg",
                    "--homedir",
                    str(home),
                    "--batch",
                    "--quiet",
                    "--import",
                    str(key_to_import),
                ],
                check=True,
                capture_output=True,
            )
        yield home


def sign_file(
    file_path: Path,
    secret_key_path: Path,
    output_path: Path | None = None,
) -> Path:
    """Sign file with GPG creating a detached signature."""
    if output_path is None:
        output_path = file_path.with_suffix(file_path.suffix + ".gpg")

    with gpg_temp_home(secret_key_path) as home:
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


def ensure_payload_assets(
    dist_dir: Path, version: str, repo_root: Path
) -> tuple[Path, Path, Path, ErofsInfo]:
    """Ensure root.erofs, UKI, and ISO exist; enforce size on compressed root."""
    root_erofs = dist_dir / f"ingot_{version}.root.erofs"
    slot_raw = dist_dir / f"ingot_{version}.slot.raw"

    if not root_erofs.exists():
        if not slot_raw.exists():
            raise FileNotFoundError(
                f"Neither {root_erofs} nor {slot_raw} found. Run tools/build.sh first."
            )
        print(f"release: extracting {root_erofs.name} from {slot_raw.name}...")
        info = extract_root_erofs(slot_raw, root_erofs)
    else:
        info = read_erofs_size(root_erofs)

    size = assert_image_size(root_erofs)
    print(
        f"release: compressed image {root_erofs.name} verified ({size} bytes, {size / 1024 / 1024:.1f} MiB < 2 GiB)"
    )

    uki = dist_dir / f"ingot_{version}.efi"
    if not uki.exists():
        raise FileNotFoundError(f"UKI not found: {uki}. Run tools/build.sh first.")

    iso = dist_dir / f"ingot_{version}.iso"
    if not iso.exists():
        raise FileNotFoundError(f"Live ISO not found: {iso} (run tools/build-iso.sh)")

    return root_erofs, uki, iso, info


def sign_payload_assets(
    payload_assets: list[Path], dist_dir: Path, secret_key: Path
) -> tuple[Path, Path, list[Path]]:
    """Compute SHA256SUMS and detached signatures for all payload assets."""
    sums_lines = []
    for asset in payload_assets:
        digest = sha256_file(asset)
        sums_lines.append(f"{digest} *{asset.name}\n")

    sums_path = dist_dir / "SHA256SUMS"
    sums_path.write_text("".join(sums_lines))
    print(f"release: generated {sums_path.name} ({len(payload_assets)} assets)")

    sums_sig = sign_file(sums_path, secret_key)
    print(f"release: signed {sums_sig.name}")

    asset_sigs = []
    for asset in payload_assets:
        sig = sign_file(asset, secret_key)
        asset_sigs.append(sig)
        print(f"release: signed {sig.name}")

    return sums_path, sums_sig, asset_sigs


def assemble_and_sign_manifest(
    version: str,
    dist_dir: Path,
    pins: dict,
    erofs_info: ErofsInfo,
    release_files: list[Path],
    secret_key: Path,
) -> tuple[Path, Path]:
    """Assemble manifest.json from build metadata and sign it."""
    build_meta = load_build_metadata(dist_dir / f"build-metadata-{version}.json")
    erofs_params = build_meta.get("erofs_parameters", {})

    erofs_dict = {
        "source_date_epoch": erofs_params.get("source_date_epoch", 1789516800),
        "compression": erofs_params.get("compression", "zstd"),
        "block_size": erofs_info.blksize,
        "blocks": erofs_info.blocks,
        "bytes": erofs_info.total_bytes,
    }
    if "mechanism" in erofs_params:
        erofs_dict["mechanism"] = erofs_params["mechanism"]

    roles = {
        f"ingot_{version}.root.erofs": "payload",
        f"ingot_{version}.efi": "uki",
        f"ingot_{version}.iso": "live",
    }
    manifest_assets = []
    for f in release_files:
        entry = {"name": f.name, "sha256": sha256_file(f)}
        if f.name in roles:
            entry["role"] = roles[f.name]
        manifest_assets.append(entry)

    manifest = build_manifest(
        version=version,
        pins=pins,
        erofs_info=erofs_dict,
        kernel_nvr=build_meta.get("kernel", "unknown"),
        assets=manifest_assets,
    )
    manifest_path = dist_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(
        f"release: generated {manifest_path.name} (v1 schema, {len(manifest_assets)} assets)"
    )

    manifest_sig = sign_file(manifest_path, secret_key)
    print(f"release: signed {manifest_sig.name}")
    return manifest_path, manifest_sig


def verify_signatures_set(
    dist_dir: Path, signatures: list[Path], pubkey_path: Path
) -> None:
    """Verify that every signature matches its target asset."""
    for sig in signatures:
        target_name = sig.name.removesuffix(".gpg")
        target = dist_dir / target_name
        if not verify_file_signature(target, sig, pubkey_path):
            raise RuntimeError(
                f"Signature verification failed for {sig.name} against {pubkey_path}"
            )
        print(f"release: verified signature {sig.name} -> PASS")


def build_release(
    version: str | None = None,
    dist_dir: Path | None = None,
    repo_root: Path = REPO_ROOT,
    secret_key_path: Path | None = None,
) -> dict:
    """Execute the release build pipeline."""
    if dist_dir is None:
        dist_dir = repo_root / "dist"
    pins = json.loads((repo_root / "tools/pins.json").read_text())
    if version is None:
        version = pins["image_version"]

    if secret_key_path is None:
        secret_key_path = repo_root / "tools/keys/project.sec"
    pubkey_path = repo_root / "tools/keys/project.pgp"

    if not secret_key_path.exists():
        raise FileNotFoundError(f"Project secret key not found at {secret_key_path}")
    if not pubkey_path.exists():
        raise FileNotFoundError(f"Project public key not found at {pubkey_path}")

    root_erofs, uki, iso, erofs_info = ensure_payload_assets(
        dist_dir, version, repo_root
    )
    payload_assets = [root_erofs, uki, iso]
    sums_path, sums_sig, asset_sigs = sign_payload_assets(
        payload_assets, dist_dir, secret_key_path
    )

    intermediate_files = payload_assets + asset_sigs + [sums_path, sums_sig]
    manifest_path, manifest_sig = assemble_and_sign_manifest(
        version=version,
        dist_dir=dist_dir,
        pins=pins,
        erofs_info=erofs_info,
        release_files=intermediate_files,
        secret_key=secret_key_path,
    )

    all_sigs = asset_sigs + [sums_sig, manifest_sig]
    verify_signatures_set(dist_dir, all_sigs, pubkey_path)

    all_artifacts = intermediate_files + [manifest_path, manifest_sig]
    print("\nrelease: all assets produced and GPG signatures verified successfully")
    return {
        "version": version,
        "assets": [f.name for f in all_artifacts],
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
            version=args.version, dist_dir=args.dist, secret_key_path=args.key
        )
    except (RuntimeError, ValueError, FileNotFoundError, OSError) as e:
        print(f"release error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
