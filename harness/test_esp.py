"""Unit tests for esp.py (ESP carve/restore + mtools operations).

Scratch disks are built with the pinned sgdisk + dosfstools (mutation by
pinned host tools, verified by our own reader - the harness's standard
split): a small GPT disk with a real FAT ESP, so the mtools round-trips
exercise the same code path the A/B scenario uses.
"""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import esp
import gpt

SECTOR = 512
ESP_LBA = 2048  # 1 MiB
ESP_SECTORS = 64 * 1024 * 1024 // SECTOR  # 64 MiB
USR_SECTORS = 64 * 1024 * 1024 // SECTOR  # 64 MiB
DISK_SECTORS = ESP_LBA + ESP_SECTORS + USR_SECTORS + 34  # + backup GPT
ESP_UUID = "12345678-9abc-def0-1234-56789abcdef0"


def build_scratch_disk(path: Path) -> None:
    """GPT disk: esp (FAT32) + usr, esp at the real first LBA."""
    path.write_bytes(b"\x00" * (DISK_SECTORS * SECTOR))
    subprocess.run([
        "sgdisk",
        f"-n 1:{ESP_LBA}:{ESP_LBA + ESP_SECTORS - 1}",
        "-t 1:0xef",
        f"-c 1:esp",
        f"-u 1:{ESP_UUID}",
        f"-n 2:{ESP_LBA + ESP_SECTORS}:{DISK_SECTORS - 34}",
        "-c 2:usr",
        "-u 2:00000000-0000-0000-0000-000000000000",
        str(path),
    ], check=True, capture_output=True)
    # a real FAT ESP where the carved image will live: carve the region
    # with dd, format it, write it back (dd of= truncates, so notrunc)
    esp_img = path.with_suffix(".espimg")
    subprocess.run(
        ["dd", f"if={path}", f"of={esp_img}", "bs=512",
         f"skip={ESP_LBA}", f"count={ESP_SECTORS}", "status=none"],
        check=True, capture_output=True)
    subprocess.run(["mkfs.fat", "-F", "32", "-s", "32", str(esp_img)],
                   check=True, capture_output=True)
    subprocess.run(
        ["dd", f"if={esp_img}", f"of={path}", "bs=512",
         f"seek={ESP_LBA}", "conv=notrunc", "status=none"],
        check=True, capture_output=True)
    esp_img.unlink()


def mkfat_dir(img: Path, rel: str) -> None:
    """mtools mkdir -p (mmd; each level, existing dirs are not errors)."""
    parts = rel.strip("/").split("/")
    for i in range(1, len(parts) + 1):
        d = "/".join(parts[:i])
        subprocess.run(
            ["mmd", "-i", str(img), "::" + d],
            capture_output=True, check=False)


class TestCarveRestore(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="esp-test-"))
        self.disk = self.tmp / "disk.img"
        build_scratch_disk(self.disk)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_carve_is_byte_exact(self):
        img = self.tmp / "esp.img"
        esp.carve_esp(self.disk, img)
        g = gpt.read_gpt(self.disk)
        e = g.by_label("esp")
        self.assertEqual(len(img.read_bytes()), e.size_bytes)
        with open(self.disk, "rb") as f:
            f.seek(e.first_lba * SECTOR)
            self.assertEqual(img.read_bytes(), f.read(e.size_bytes))

    def test_restore_roundtrip(self):
        img = self.tmp / "esp.img"
        esp.carve_esp(self.disk, img)
        data = img.read_bytes()
        esp.restore_esp(self.disk, img)
        with open(self.disk, "rb") as f:
            g = gpt.read_gpt(self.disk)
            e = g.by_label("esp")
            f.seek(e.first_lba * SECTOR)
            self.assertEqual(f.read(e.size_bytes), data)

    def test_restore_refuses_size_mismatch(self):
        img = self.tmp / "esp.img"
        esp.carve_esp(self.disk, img)
        small = self.tmp / "small.img"
        small.write_bytes(img.read_bytes()[:1024])
        with self.assertRaises(esp.EspError):
            esp.restore_esp(self.disk, small)

    def test_missing_label_raises(self):
        with self.assertRaises(esp.EspError):
            esp.carve_esp(self.disk, self.tmp / "x.img", label="nope")


class TestMtoolsRoundTrip(unittest.TestCase):
    """Write, read, and rename on a real FAT ESP via the carved image."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="esp-mtools-"))
        self.disk = self.tmp / "disk.img"
        build_scratch_disk(self.disk)
        self.img = self.tmp / "esp.img"
        esp.carve_esp(self.disk, self.img)
        mkfat_dir(self.img, "EFI/Linux")

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_write_read(self):
        payload = b"ingot uki payload \x00\x01\x02"
        esp.esp_write(self.img, "EFI/Linux/ingot_0.2.0+3.efi", payload)
        self.assertEqual(
            esp.esp_read(self.img, "EFI/Linux/ingot_0.2.0+3.efi"), payload)

    def test_rename(self):
        esp.esp_write(self.img, "EFI/Linux/ingot_0.2.0.efi", b"uki")
        esp.esp_rename(self.img, "EFI/Linux/ingot_0.2.0.efi",
                       "EFI/Linux/ingot_0.2.0+3.efi")
        self.assertIn("::/EFI/Linux/ingot_0.2.0+3.efi", esp.esp_list(self.img))
        self.assertNotIn("::/EFI/Linux/ingot_0.2.0.efi", esp.esp_list(self.img))

    def test_files_and_forensics_after_restore(self):
        esp.esp_write(self.img, "EFI/Linux/ingot_0.1.0.efi", b"uki-a")
        esp.esp_write(self.img, "EFI/Linux/ingot_0.2.0+3.efi", b"uki-b")
        esp.restore_esp(self.disk, self.img)
        # re-carve from the disk: the writes persisted through the round-trip
        img2 = self.tmp / "esp2.img"
        esp.carve_esp(self.disk, img2)
        files = esp.esp_files(img2)
        self.assertEqual(set(files), {"ingot_0.1.0", "ingot_0.2.0+3"})
        state = esp.esp_forensics(img2)
        self.assertEqual(state["states"]["ingot_0.1.0"], "good")
        self.assertEqual(state["states"]["ingot_0.2.0+3"], "indeterminate")
        self.assertEqual(state["counters"]["ingot_0.2.0+3"], [3, 0])
        self.assertEqual(state["default"], "ingot_0.2.0+3")


if __name__ == "__main__":
    unittest.main()
