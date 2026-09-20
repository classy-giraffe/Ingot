"""Unit tests for tools.publish (T7: Publish gate).

Acceptance criteria:
- The publish step refuses to run without a green harness result;
  the published release is immutable.
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import publish


class TestPublishGate(unittest.TestCase):
    def test_harness_gate_missing_file(self):
        """Publish refuses to run if harness results file does not exist."""
        with tempfile.TemporaryDirectory() as td:
            missing_path = Path(td) / "nonexistent-results.json"
            with self.assertRaises(RuntimeError) as ctx:
                publish.check_harness_gate(missing_path)
            self.assertIn("missing harness results", str(ctx.exception).lower())

    def test_harness_gate_failed_overall(self):
        """Publish refuses to run if pass is False."""
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            f.write(json.dumps({"pass": False, "checks": {}}).encode())
            results_path = Path(f.name)

        try:
            with self.assertRaises(RuntimeError) as ctx:
                publish.check_harness_gate(results_path)
            self.assertIn("harness gate failed", str(ctx.exception).lower())
        finally:
            results_path.unlink()

    def test_harness_gate_empty_checks_fails(self):
        """Publish refuses to run if checks dictionary is empty."""
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            f.write(json.dumps({"pass": True, "checks": {}}).encode())
            results_path = Path(f.name)

        try:
            with self.assertRaises(RuntimeError) as ctx:
                publish.check_harness_gate(results_path)
            self.assertIn("no check entries", str(ctx.exception).lower())
        finally:
            results_path.unlink()

    def test_harness_gate_failed_individual_check(self):
        """Publish refuses to run if any individual check is not passing."""
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            data = {
                "pass": True,
                "checks": {
                    "uefi_secure_boot": {"ok": True},
                    "usr_slot_ro": {"ok": False, "detail": "failed"},
                },
            }
            f.write(json.dumps(data).encode())
            results_path = Path(f.name)

        try:
            with self.assertRaises(RuntimeError) as ctx:
                publish.check_harness_gate(results_path)
            self.assertIn("usr_slot_ro", str(ctx.exception))
        finally:
            results_path.unlink()

    def test_harness_gate_green(self):
        """Publish proceeds when harness results are green."""
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            data = {
                "pass": True,
                "checks": {
                    "uefi_secure_boot": {"ok": True},
                    "usr_slot_ro": {"ok": True},
                },
            }
            f.write(json.dumps(data).encode())
            results_path = Path(f.name)

        try:
            ok = publish.check_harness_gate(results_path)
            self.assertTrue(ok)
        finally:
            results_path.unlink()

    @patch("subprocess.run")
    def test_immutability_refuses_existing_release(self, mock_run):
        """Publish refuses to run if the release already exists on GitHub (immutable)."""
        # mock gh release view returning 0 (release already exists)
        mock_run.return_value = MagicMock(returncode=0, stdout="v0.1.0\n", stderr="")
        with self.assertRaises(RuntimeError) as ctx:
            publish.check_release_immutability("classy-giraffe/Ingot", "v0.1.0")
        err_msg = str(ctx.exception).lower()
        self.assertIn("already exists", err_msg)
        self.assertIn("immutable", err_msg)

    @patch("subprocess.run")
    def test_immutability_allows_new_release(self, mock_run):
        """Publish allows release creation if the tag does not exist yet."""
        # mock gh release view returning 1 (not found)
        mock_run.return_value = MagicMock(
            returncode=1, stdout="", stderr="release not found"
        )
        ok = publish.check_release_immutability("classy-giraffe/Ingot", "v0.1.0")
        self.assertTrue(ok)

    @patch("subprocess.run")
    def test_immutability_raises_on_network_or_auth_error(self, mock_run):
        """Publish fails if checking release fails with network or auth error, not not-found."""
        mock_run.return_value = MagicMock(
            returncode=1, stdout="", stderr="error: network connection timed out"
        )
        with self.assertRaises(RuntimeError) as ctx:
            publish.check_release_immutability("classy-giraffe/Ingot", "v0.1.0")
        self.assertIn("network connection timed out", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
