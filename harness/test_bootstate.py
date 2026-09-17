"""Unit tests for bootstate.py (machine-readable boot selection state)."""
import struct
import unittest

import bootstate


class TestCounters(unittest.TestCase):
    def test_good_entry_has_no_counters(self):
        self.assertEqual(bootstate.parse_counters("ingot_0.1.0"), (None, None))
        self.assertEqual(bootstate.parse_counters("ingot_0.1.0.efi"), (None, None))

    def test_armed_entry(self):
        self.assertEqual(bootstate.parse_counters("ingot_0.2.0+3"), (3, 0))

    def test_decremented_entry(self):
        self.assertEqual(bootstate.parse_counters("ingot_0.2.0+2-1"), (2, 1))
        self.assertEqual(bootstate.parse_counters("ingot_0.2.0+1-2"), (1, 2))

    def test_exhausted_entry(self):
        self.assertEqual(bootstate.parse_counters("ingot_0.2.2+0-3"), (0, 3))

    def test_classify(self):
        self.assertEqual(bootstate.classify("ingot_0.1.0"), "good")
        self.assertEqual(bootstate.classify("ingot_0.2.0+3"), "indeterminate")
        self.assertEqual(bootstate.classify("ingot_0.2.0+1-2"), "indeterminate")
        self.assertEqual(bootstate.classify("ingot_0.2.0+0-3"), "bad")


class TestDefaultEntry(unittest.TestCase):
    def test_newest_non_bad_wins(self):
        self.assertEqual(
            bootstate.default_entry(["ingot_0.1.0", "ingot_0.2.0"]), "ingot_0.2.0"
        )

    def test_armed_new_entry_is_selected(self):
        # a freshly deployed (armed) slot is booted; protection is the
        # counter, not exclusion
        self.assertEqual(
            bootstate.default_entry(["ingot_0.1.0", "ingot_0.2.0+3"]),
            "ingot_0.2.0+3",
        )

    def test_bad_new_entry_falls_back(self):
        self.assertEqual(
            bootstate.default_entry(["ingot_0.1.0", "ingot_0.2.0+0-3"]),
            "ingot_0.1.0",
        )

    def test_only_bad_entry_is_still_selected(self):
        self.assertEqual(bootstate.default_entry(["ingot_0.2.0+0-3"]), "ingot_0.2.0+0-3")

    def test_versionsort_is_numeric_not_lexicographic(self):
        self.assertEqual(
            bootstate.default_entry(["ingot_0.1.9", "ingot_0.1.10"]),
            "ingot_0.1.10",
        )

    def test_empty(self):
        self.assertIsNone(bootstate.default_entry([]))


def _make_pe_with_osrel(content: bytes) -> bytes:
    """A minimal PE32+ image carrying one .osrel section.

    The PE header block starts at e_lfanew (64), so COFF, optional
    header, and section table land at the offsets the parser walks.
    """
    e_lfanew = 64
    opt_size = 240
    nsec = 1
    coff_off = e_lfanew + 4
    opt_off = coff_off + 20
    sect_off = opt_off + opt_size
    data_off = sect_off + 40
    total = max(128, data_off + len(content))
    img = bytearray(total)
    struct.pack_into("<I", img, 0x3C, e_lfanew)
    img[e_lfanew:e_lfanew + 4] = b"PE\x00\x00"
    # COFF header: machine, nsections, time, ptr_symtab, nsym, size_opt, ch
    struct.pack_into("<HHIIIHH", img, coff_off,
                     0x8664, nsec, 0, 0, 0, opt_size, 0)
    # optional header (PE32+): magic, then zeros
    struct.pack_into("<H", img, opt_off, 0x20B)
    # section table: .osrel
    img[sect_off:sect_off + 8] = b".osrel\x00\x00\x00"
    struct.pack_into("<IIIIIIHHI", img, sect_off + 8,
                     len(content),   # virtual size
                     0x1000,         # virtual address
                     len(content),   # size of raw data
                     data_off,       # pointer to raw data
                     0, 0, 0, 0,     # relocs / linenums
                     0x40000040)     # characteristics: code|read
    img[data_off:data_off + len(content)] = content
    return bytes(img)


class TestOsrel(unittest.TestCase):
    def test_version_id(self):
        content = (
            b"NAME=Ingot\n"
            b"VERSION=0.2.0 (workstation)\n"
            b"VERSION_ID=0.2.0\n"
            b"OSRELEASEREVISION=1\n"
        )
        self.assertEqual(bootstate.osrel_version(_make_pe_with_osrel(content)), "0.2.0")

    def test_no_osrel_section(self):
        # a PE with no .osrel section (empty section table is not valid PE;
        # use a non-PE blob instead: must not raise)
        self.assertIsNone(bootstate.osrel_version(b"not a PE file at all"))
        self.assertIsNone(bootstate.osrel_version(b""))


class TestLoaderConf(unittest.TestCase):
    def test_parse(self):
        text = (
            "# comment line\n"
            "timeout 5\n"
            "auto-continue 1\n"
            "\n"
            "default ingot_0.2.0\n"
        )
        conf = bootstate.parse_loader_conf(text)
        self.assertEqual(conf["timeout"], "5")
        self.assertEqual(conf["auto-continue"], "1")
        self.assertEqual(conf["default"], "ingot_0.2.0")

    def test_empty(self):
        self.assertEqual(bootstate.parse_loader_conf(""), {})


class TestSelectionState(unittest.TestCase):
    def test_full_state(self):
        versions = {"ingot_0.1.0": "0.1.0", "ingot_0.2.0": "0.2.0"}
        state = bootstate.selection_state(
            ["ingot_0.1.0", "ingot_0.2.0+0-3"],
            {"timeout": "5"},
            versions,
        )
        self.assertEqual(state["default"], "ingot_0.1.0")
        by_id = {e["id"]: e for e in state["entries"]}
        self.assertEqual(by_id["ingot_0.1.0"]["state"], "good")
        self.assertTrue(by_id["ingot_0.1.0"]["blessed"])
        self.assertEqual(by_id["ingot_0.2.0+0-3"]["state"], "bad")
        self.assertEqual(by_id["ingot_0.2.0+0-3"]["tries_left"], 0)
        self.assertEqual(by_id["ingot_0.2.0+0-3"]["attempts"], 3)
        self.assertEqual(by_id["ingot_0.2.0+0-3"]["version"], "0.2.0")
        self.assertFalse(by_id["ingot_0.2.0+0-3"]["blessed"])
        self.assertEqual(state["loader_conf"], {"timeout": "5"})

    def test_armed_entry_counters(self):
        state = bootstate.selection_state(["ingot_0.2.0+3"], {}, {})
        entry = state["entries"][0]
        self.assertEqual(entry["state"], "indeterminate")
        self.assertEqual(entry["tries_left"], 3)
        self.assertEqual(entry["attempts"], 0)
        self.assertIsNone(entry.get("version"))


if __name__ == "__main__":
    unittest.main()
