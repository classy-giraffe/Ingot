"""Unit tests for prep_var.py (probe fixture layout).

The root-only loop-mount path is not exercised here (it is a thin
subprocess wrapper around mount/umount); the fixture layout that the
guest's systemd must find is the load-bearing logic and is pure
filesystem state under a mountpoint - testable with a tmpdir.
"""
import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path

import prep_var

PROBE_DIR = Path(__file__).resolve().parent / "probe"


class TestInject(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="prep-test-"))

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_layout(self):
        prep_var.inject(self.tmp, PROBE_DIR)
        # self.tmp stands in for the /var partition's mountpoint (the
        # partition is mounted at /var at runtime): the fixture must
        # land at the partition-root-relative paths, and must not
        # create a stray "var/" subdirectory of /var.
        self.assertFalse((self.tmp / "var").exists())
        unit = self.tmp / "lib/etc/systemd/system" / prep_var.UNIT_NAME
        wants = (self.tmp / "lib/etc/systemd/system"
                 / "multi-user.target.wants" / prep_var.UNIT_NAME)
        script = self.tmp / "lib/ingot-probe/harness-probe.sh"

        self.assertTrue(unit.is_file())
        self.assertEqual(stat.S_IMODE(unit.stat().st_mode), 0o644)
        self.assertEqual(unit.read_bytes(),
                         (PROBE_DIR / prep_var.UNIT_NAME).read_bytes())

        self.assertTrue(script.is_file())
        self.assertEqual(stat.S_IMODE(script.stat().st_mode), 0o755)
        self.assertEqual(script.read_bytes(),
                         (PROBE_DIR / "harness-probe.sh").read_bytes())

        # a real machine-id keeps the scenario disk out of systemd's
        # first-boot flow (preset policy + interactive firstboot)
        machine_id = self.tmp / "lib/etc/machine-id"
        self.assertTrue(machine_id.is_file())
        self.assertEqual(stat.S_IMODE(machine_id.stat().st_mode), 0o444)
        self.assertEqual(machine_id.read_text(), prep_var.MACHINE_ID)
        self.assertRegex(prep_var.MACHINE_ID, r"^[0-9a-f]{32}$")
        self.assertNotEqual(prep_var.MACHINE_ID, "0" * 32)

    def test_idempotent_reinject(self):
        prep_var.inject(self.tmp, PROBE_DIR)
        first = (self.tmp / "lib/etc/systemd/system"
                 / prep_var.UNIT_NAME).read_bytes()
        prep_var.inject(self.tmp, PROBE_DIR)  # second boot's re-injection
        unit = (self.tmp / "lib/etc/systemd/system"
                / prep_var.UNIT_NAME)
        wants = (self.tmp / "lib/etc/systemd/system"
                / "multi-user.target.wants" / prep_var.UNIT_NAME)
        self.assertEqual(unit.read_bytes(), first)
        self.assertTrue(wants.is_symlink())
        self.assertEqual((self.tmp / "lib/etc/machine-id").read_text(),
                         prep_var.MACHINE_ID)


if __name__ == "__main__":
    unittest.main()
