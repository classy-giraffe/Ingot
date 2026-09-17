"""Unit tests for gpt.py (pure-Python GPT reader).

Scratch disks are built with the pinned sgdisk (mutation by a pinned host
tool, verified by our own reader - the harness's standard split).
"""
import shutil
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path

import gpt

SECTOR = 512
DISK_SIZE = 64 * 1024 * 1024  # 64 MiB scratch disk


def build_scratch_disk(path: Path) -> None:
    """A 3-partition GPT disk: esp (1M) + usr (16M) + var (16M)."""
    path.write_bytes(b"\x00" * DISK_SIZE)
    subprocess.run(
        [
            "sgdisk",
            "-Z",
            "-n", "1:2048:4095", "-c", "1:esp",
            "-t", "1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
            "-n", "2:4096:36863", "-c", "2:usr",
            "-n", "3:36864:69631", "-c", "3:var",
            str(path),
        ],
        check=True,
        capture_output=True,
    )


class TestReadGpt(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="gpt-test-"))
        self.disk = self.tmp / "scratch.raw"
        build_scratch_disk(self.disk)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_entries_parsed(self):
        g = gpt.read_gpt(self.disk)
        self.assertEqual(len(g.entries), 3)
        labels = [e.label for e in g.entries]
        self.assertEqual(labels, ["esp", "usr", "var"])
        # esp starts at the standard 2048 (1 MiB offset)
        self.assertEqual(g.entries[0].first_lba, 2048)
        self.assertEqual(g.entries[0].last_lba, 4095)
        self.assertEqual(g.entries[0].size_bytes, (4095 - 2048 + 1) * SECTOR)
        for e in g.entries:
        # PARTUUIDs are lowercase UUID strings
            self.assertEqual(len(e.part_uuid), 36)
            self.assertEqual(e.part_uuid, e.part_uuid.lower())

    def test_lookups(self):
        g = gpt.read_gpt(self.disk)
        usr = g.by_label("usr")
        self.assertEqual(usr.first_lba, 4096)
        self.assertIs(g.by_part_uuid(usr.part_uuid), usr)

    def test_lookup_misses_raise(self):
        g = gpt.read_gpt(self.disk)
        with self.assertRaises(gpt.GptError):
            g.by_label("nope")
        with self.assertRaises(gpt.GptError):
            g.by_part_uuid("00000000-0000-0000-0000-000000000000")

    def test_entry_array_corruption_detected(self):
        data = bytearray(self.disk.read_bytes())
        # clobber the 'esp' label area in the PRIMARY entry array
        # (entries at LBA 2; entry 0 name field at offset 56)
        off = 2 * SECTOR + 56
        data[off] = ord("X")
        bad = self.tmp / "bad-entries.raw"
        bad.write_bytes(bytes(data))
        with self.assertRaises(gpt.GptError):
            gpt.read_gpt(bad)

    def test_header_corruption_detected(self):
        data = bytearray(self.disk.read_bytes())
        # clobber the first-usable-LBA field of the primary header
        # (header at LBA 1, field at header offset 40)
        data[1 * SECTOR + 40 + 1] = 0xFF
        bad = self.tmp / "bad-header.raw"
        bad.write_bytes(bytes(data))
        with self.assertRaises(gpt.GptError):
            gpt.read_gpt(bad)

    def test_too_small(self):
        tiny = self.tmp / "tiny.raw"
        tiny.write_bytes(b"\x00" * (10 * SECTOR))
        with self.assertRaises(gpt.GptError):
            gpt.read_gpt(tiny)


if __name__ == "__main__":
    unittest.main()
