"""Unit tests for console.py (console evidence parsing).

Fixtures are modeled on a real T1 serial capture: kernel log prefixes,
the manager's ShowStatus lines, and journal-forwarded copies of service
output. No dependency on dist/ artifacts.
"""
import unittest

import console

PROBE_DOC = (
    '{\n'
    '  "kernel": "7.3.0-1.fc46.x86_64",\n'
    '  "multi_user": "active",\n'
    '  "usr": {"fstype": "erofs", "partuuid": "0066bfe5-47f1-52dc-9a16-1bb10191a1dc"}\n'
    '}'
)


def frame(text: str) -> str:
    return (
        console.PROBE_BEGIN + "\n"
        + text + "\n"
        + console.PROBE_END
    )


class TestExtractProbeFrame(unittest.TestCase):
    def test_clean_frame(self):
        text = "boot noise\n" + frame(PROBE_DOC) + "\ntrailing\n"
        doc = console.extract_probe_frame(text)
        self.assertEqual(doc["multi_user"], "active")
        self.assertEqual(doc["usr"]["fstype"], "erofs")

    def test_kernel_prefix_and_ansi_interleave(self):
        # journal-forwarded copy: every line carries the kernel log
        # prefix; a kernel line interleaves inside the frame.
        lines = [
            "[    6.055131] sh[662]: " + console.PROBE_BEGIN,
            "[    6.055131] sh[662]: {",
            "\x1b[0;32m[  OK  \x1b[0m] Some status line that is not JSON",
            "[    6.055131] sh[662]:   \"kernel\": \"7.3.0-1.fc46.x86_64\",",
            "[    6.055131] sh[662]:   \"multi_user\": \"active\",",
            "[    6.056000] kernel: unrelated kernel line",
            "[    6.055131] sh[662]:   \"usr\": {\"fstype\": \"erofs\", \"partuuid\": \"0066bfe5-47f1-52dc-9a16-1bb10191a1dc\"}",
            "[    6.055131] sh[662]: }",
            "[    6.055131] sh[662]: " + console.PROBE_END,
        ]
        doc = console.extract_probe_frame("\n".join(lines))
        self.assertIsNotNone(doc)
        self.assertEqual(doc["multi_user"], "active")

    def test_getty_terminal_init_interleave(self):
        # Regression: the serial getty's interactive shell injects its
        # terminal-init bytes (NUL/ETX/form-feed, OSC set-xterm-title,
        # ESC 7/8 cursor save/restore, CSI) right before a payload line,
        # which used to make the parser drop the line and lose the key.
        getty = (
            "\x00\x00\x00\x03\x0c\x00\x00\x00\x01 \x00\x00\x00\x02 "
            "\x1b[!p\x1b[?7h\x1b[0m\x1b]104\x1b\\\x1b[1G\x1b8\x1b[0J"
        )
        lines = [
            console.PROBE_BEGIN,
            "{",
            '  "multi_user": "active",',
            getty + '  "etc": {"e:is_mount": 1, "source": "/dev/vda4", "fstab_nonempty": 1},',
            '  "usr": {"fstype": "erofs"}',
            "}",
            console.PROBE_END,
        ]
        doc = console.extract_probe_frame("\n".join(lines))
        self.assertIsNotNone(doc)
        self.assertEqual(doc["etc"]["source"], "/dev/vda4")

    def test_missing_frame(self):
        self.assertIsNone(console.extract_probe_frame("no frame here"))

    def test_open_frame_without_end(self):
        text = console.PROBE_BEGIN + "\n" + PROBE_DOC
        self.assertIsNone(console.extract_probe_frame(text))

    def test_corrupt_json(self):
        self.assertIsNone(console.extract_probe_frame(frame("{ this is not json")))


class TestBaselineParsing(unittest.TestCase):
    CONSOLE = (
        "[    0.000000] Command line: root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec "
        "rootfstype=btrfs usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc usrfstype=erofs\n"
        "[    0.512000] Secure boot enabled\n"
        "[    1.462539] systemd[1]: Successfully made /usr/ read-only.\n"
        "[  OK  ] Finished ingot-root.service - Ingot runtime root (tmpfs / with slot /usr and state /var).\n"
        "[  OK  ] Reached target multi-user.target - Multi-User System.\n"
        "[    6.055131] systemd[1]: Reached target multi-user.target - Multi-User System.\n"
        "[  OK  ] Stopped target multi-user.target - Multi-User System.\n"
    )

    def test_kernel_cmdline(self):
        cl = console.kernel_cmdline(self.CONSOLE)
        self.assertIsNotNone(cl)
        self.assertIn("root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec", cl)
        self.assertIn("usrfstype=erofs", cl)

    def test_kernel_cmdline_absent(self):
        self.assertIsNone(console.kernel_cmdline("nothing relevant"))

    def test_reached_target(self):
        self.assertTrue(console.reached_target(self.CONSOLE, "multi-user.target"))

    def test_stopped_is_not_reached(self):
        text = "[  OK  ] Stopped target multi-user.target - Multi-User System.\n"
        self.assertFalse(console.reached_target(text, "multi-user.target"))

    def test_secure_boot_enabled(self):
        self.assertTrue(console.secure_boot_enabled(self.CONSOLE))
        self.assertFalse(console.secure_boot_enabled("no sb line"))

    def test_runtime_root_prepared(self):
        self.assertTrue(console.runtime_root_prepared(self.CONSOLE))
        self.assertFalse(console.runtime_root_prepared("no prep line"))

    def test_failed_units(self):
        text = self.CONSOLE + (
            "\x1b[0;31m[FAILED]\x1b[0m Failed to start ingot-root.service - Ingot runtime root.\n"
            "[    5.000000] ingot-root.service[123]: Failed to start ingot-root.service - Ingot runtime root.\n"
        )
        self.assertEqual(console.failed_units(text), ["ingot-root.service"])
        self.assertEqual(console.failed_units(self.CONSOLE), [])

    def test_failure_markers(self):
        text = (
            "[    5.123456] ingot-root.service[123]: ingot-prepare: "
            "slot device /dev/disk/by-partuuid/abc not found\n"
        )
        markers = console.failure_markers(text)
        self.assertTrue(any("ingot-prepare:" in m for m in markers))
        self.assertEqual(console.failure_markers(self.CONSOLE), [])

    def test_console_contains_strips_ansi(self):
        # Real T2 failure capture: the manager's status line wraps the
        # unit ID in ANSI color escapes, so a raw substring match
        # misses it; console_contains matches the stripped form.
        text = (
            "[\x1b[0;1;31mFAILED\x1b[0m] Failed to start "
            "\x1b[0;1;39mingot-root.service\x1b[0m…s / with slot /usr and state /var).\n"
            "[   14.458025] erofs (device vda3): cannot find valid erofs superblock\n"
        )
        self.assertTrue(
            console.console_contains(text, "Failed to start ingot-root.service"))
        self.assertTrue(
            console.console_contains(text, "cannot find valid erofs superblock"))
        # a clean line is still matched (stripping is a no-op on it)
        self.assertTrue(console.console_contains("boot noise\nplain line here\n",
                                                "plain line"))
        self.assertFalse(
            console.console_contains(self.CONSOLE, "Failed to start ingot-root.service"))


if __name__ == "__main__":
    unittest.main()
