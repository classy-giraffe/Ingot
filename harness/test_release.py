"""Unit tests for tools.release (T7: Release pipeline).

Acceptance criteria:
- A release build produces all assets with GPG signatures that verify against
  the project public key.
- The manifest matches the v1 schema and records build metadata (compose pin,
  component pins, erofs parameters, timestamps).
- A release whose compressed image exceeds 2 GiB fails the build with a clear
  diagnostic.
"""

import json
import struct

# Module to test (tools/release.py)
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import release


def sample_manifest_fixtures():
    """Sample pins, erofs info, and assets for manifest tests."""
    pins = {
        "compose": {"id": "Fedora-Rawhide-20260916.n.0"},
        "rust": {
            "brush": {"repo": "r", "tag": "t", "sha": "s1"},
            "nushell": {"repo": "r", "tag": "t", "sha": "s2"},
            "helix": {"repo": "r", "tag": "t", "sha": "s3"},
            "zellij": {"repo": "r", "tag": "t", "sha": "s4"},
            "coreutils": {"repo": "r", "tag": "t", "sha": "s5"},
        },
    }
    erofs_info = {
        "source_date_epoch": 1789516800,
        "compression": "zstd",
        "block_size": 4096,
        "blocks": 137797,
        "bytes": 564416512,
    }
    assets = [
        {"name": "ingot_0.1.0.root.erofs", "sha256": "a" * 64, "role": "payload"},
        {"name": "ingot_0.1.0.efi", "sha256": "b" * 64, "role": "uki"},
        {"name": "ingot_0.1.0.iso", "sha256": "c" * 64, "role": "live"},
        {"name": "SHA256SUMS", "sha256": "d" * 64},
        {"name": "SHA256SUMS.gpg", "sha256": "e" * 64},
    ]
    return pins, erofs_info, assets


def setup_mock_release_tree(td_path: Path, files: list[Path]) -> tuple[Path, Path]:
    """Create local releases.json and assets directory for update-helper testing."""
    releases_json = td_path / "releases.json"
    releases_json.write_text(
        json.dumps([{"tag_name": "v0.1.0", "draft": False, "prerelease": False}])
    )
    assets_dir = td_path / "assets" / "v0.1.0"
    assets_dir.mkdir(parents=True)
    for f in files:
        (assets_dir / f.name).write_bytes(f.read_bytes())
    return releases_json, td_path / "assets"


class TestReleasePipeline(unittest.TestCase):
    def test_erofs_superblock_parse(self):
        """Parse erofs superblock to get block count and block size."""
        # EROFS superblock: 1024 offset, magic 0xE0F5E1E2, blkszbits at +12, blocks at +36
        data = bytearray(2048)
        struct.pack_into("<I", data, 1024, 0xE0F5E1E2)
        data[1024 + 12] = 12  # 2^12 = 4096
        struct.pack_into("<I", data, 1024 + 36, 100)  # 100 blocks

        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(data)
            f.flush()
            temp_path = Path(f.name)

        try:
            blocks, blksize, total_bytes = release.read_erofs_size(temp_path)
            self.assertEqual(blocks, 100)
            self.assertEqual(blksize, 4096)
            self.assertEqual(total_bytes, 100 * 4096)
        finally:
            temp_path.unlink()

    def test_image_size_assertion_under_2gib(self):
        """Compressed image under 2 GiB passes check."""
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.truncate(500 * 1024 * 1024)  # 500 MiB
            temp_path = Path(f.name)

        try:
            size = release.assert_image_size(temp_path)
            self.assertEqual(size, 500 * 1024 * 1024)
        finally:
            temp_path.unlink()

    def test_image_size_assertion_over_2gib_fails(self):
        """Compressed image exceeding 2 GiB fails with clear diagnostic."""
        with tempfile.NamedTemporaryFile(delete=False) as f:
            # 2 GiB + 1 byte
            over_size = 2 * 1024 * 1024 * 1024 + 1
            f.truncate(over_size)
            temp_path = Path(f.name)

        try:
            with self.assertRaises(ValueError) as ctx:
                release.assert_image_size(temp_path)
            err_msg = str(ctx.exception)
            self.assertIn("exceeds 2 GiB", err_msg)
            self.assertIn(str(over_size), err_msg)
        finally:
            temp_path.unlink()

    def test_manifest_v1_schema_and_metadata(self):
        """Manifest conforms to v1 schema with required build metadata."""
        pins, erofs_info, assets = sample_manifest_fixtures()
        manifest = release.build_manifest(
            version="0.1.0",
            pins=pins,
            erofs_info=erofs_info,
            kernel_nvr="7.3.0-0.rc3.fc46.x86_64",
            assets=assets,
            timestamp="2026-09-20T12:00:00Z",
        )

        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(manifest["version"], "0.1.0")
        self.assertEqual(manifest["tag"], "v0.1.0")
        self.assertEqual(manifest["image_version"], "0.1.0")

        build = manifest["build"]
        self.assertEqual(build["compose"], "Fedora-Rawhide-20260916.n.0")
        self.assertEqual(build["component_pins"], pins["rust"])
        self.assertEqual(build["erofs"], erofs_info)
        self.assertEqual(build["kernel"], "7.3.0-0.rc3.fc46.x86_64")
        self.assertEqual(build["timestamp"], "2026-09-20T12:00:00Z")

        self.assertEqual(len(manifest["assets"]), 5)
        for a in manifest["assets"]:
            self.assertTrue(len(a["sha256"]) == 64)

    def test_gpg_sign_and_verify(self):
        """Sign an asset with project secret key and verify with public keyring."""
        keyring = REPO_ROOT / "tools/keys/project.pgp"
        sec_key = REPO_ROOT / "tools/keys/project.sec"
        if not sec_key.exists() or not keyring.exists():
            self.skipTest("project keys not available")

        with tempfile.TemporaryDirectory() as td:
            td_path = Path(td)
            test_file = td_path / "test-asset.txt"
            test_file.write_bytes(b"hello Ingot release pipeline")

            sig_path = release.sign_file(test_file, sec_key)
            self.assertTrue(sig_path.exists())

            # Verification against public keyring must succeed
            ok = release.verify_file_signature(test_file, sig_path, keyring)
            self.assertTrue(ok)

            # Verification of tampered file must fail
            tampered_file = td_path / "tampered.txt"
            tampered_file.write_bytes(b"tampered content")
            bad = release.verify_file_signature(tampered_file, sig_path, keyring)
            self.assertFalse(bad)

    def test_update_helper_verifies_release(self):
        """update-helper authenticates manifest, sums, and emits valid pin."""
        helper_bin = REPO_ROOT / "src/target/release/ingot-update-helper"
        if not helper_bin.exists():
            helper_bin = REPO_ROOT / "src/target/debug/ingot-update-helper"
        if not helper_bin.exists():
            self.skipTest("ingot-update-helper binary not built")

        keyring = REPO_ROOT / "tools/keys/project.pgp"
        manifest_file = REPO_ROOT / "dist/manifest.json"
        manifest_sig = REPO_ROOT / "dist/manifest.json.gpg"
        sums_file = REPO_ROOT / "dist/SHA256SUMS"
        sums_sig = REPO_ROOT / "dist/SHA256SUMS.gpg"

        for f in (keyring, manifest_file, manifest_sig, sums_file, sums_sig):
            if not f.exists():
                self.skipTest(f"required file {f.name} missing for helper test")

        import subprocess

        with tempfile.TemporaryDirectory() as td:
            td_path = Path(td)
            files = [manifest_file, manifest_sig, sums_file, sums_sig]
            rel_json, asset_base = setup_mock_release_tree(td_path, files)
            cmd = [
                str(helper_bin),
                "--repo",
                "classy-giraffe/Ingot",
                "--keyring",
                str(keyring),
                "--api",
                f"file://{rel_json}",
                "--asset-base",
                f"file://{asset_base}",
            ]
            res = subprocess.run(cmd, capture_output=True, text=True, check=False)
            self.assertEqual(res.returncode, 0, f"helper stderr: {res.stderr}")
            pin = json.loads(res.stdout)
            self.assertEqual(pin["version"], "0.1.0")
            self.assertEqual(pin["tag"], "v0.1.0")
            self.assertTrue(len(pin["assets"]) >= 3)

    def test_load_build_metadata(self):
        """Dynamic metadata loading extracts kernel, compose, and erofs parameters."""
        meta_data = {
            "image_id": "ingot",
            "image_version": "0.1.0",
            "compose": "Fedora-Rawhide-test",
            "kernel": "7.3.0-custom",
            "artifacts": {
                "slot_erofs": {
                    "parameters": {
                        "source_date_epoch": 123456789,
                        "compression": "zstd",
                        "mechanism": "custom-repart",
                    }
                }
            },
        }
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            f.write(json.dumps(meta_data).encode())
            meta_path = Path(f.name)

        try:
            meta = release.load_build_metadata(meta_path)
            self.assertEqual(meta["kernel"], "7.3.0-custom")
            self.assertEqual(meta["compose"], "Fedora-Rawhide-test")
            self.assertEqual(meta["erofs_parameters"]["mechanism"], "custom-repart")
            self.assertEqual(meta["erofs_parameters"]["source_date_epoch"], 123456789)
        finally:
            meta_path.unlink()


if __name__ == "__main__":
    unittest.main()
