"""Unit tests for run.t1_checks (T1 boot invariants, probe mode).

The admin-userland checks (issue #21): criterion 9 (brush is /bin/sh,
the 24.1 shell-compat smoke), criterion 10 (nushell is the default
interactive shell), and the editor/multiplexer launch checks read the
probe document's sh / nu_ok / helix_ok / zellij_ok fields. A probe
that lacks a field must fail the check, never pass silently.
"""
import unittest

import run


def green_probe(**overrides):
    """A probe document where every T1 invariant holds."""
    v = run.PINS_VERSION
    p = {
        "kernel": "7.3.0-test",
        "version": v,
        "pid1_comm": "systemd",
        "pid1_exe": "/usr/lib/systemd/systemd",
        "cmdline": (
            "console=tty0 console=ttyS0 "
            f"root=PARTUUID={run.STATE_UUID} rootfstype=btrfs "
            f"usr=PARTUUID={run.SLOT_A_UUID} usrfstype=erofs"
        ),
        "secure_boot": 1,
        "efivars_sb": 1,
        "sysfs_efi": "",
        "loader_features": "",
        "root": {"fstype": "tmpfs"},
        "usr": {
            "source": "/dev/vda2",
            "fstype": "erofs",
            "options": "ro,relatime",
            "partuuid": run.SLOT_A_UUID,
        },
        "var": {"fstype": "btrfs", "source": "/dev/vda4"},
        "etc": {"is_mount": 1, "source": "/dev/vda4", "fstab_nonempty": 1},
        "multi_user": "active",
        "system_running": "running",
        "failed_units": "",
        "journal_err": "",
        "sh": "/usr/bin/brush",
        "helper_ok": 1,
        "nu_ok": 1,
        "helix_ok": 1,
        "zellij_ok": 1,
        "microcode_rev": "unknown",
    }
    p.update(overrides)
    return p


def ev_and_esp():
    ev = {"console": "", "secure_boot": True}
    esp = {"entries": [f"ingot_{run.PINS_VERSION}"],
           "default": f"ingot_{run.PINS_VERSION}"}
    return ev, esp


class TestT1Checks(unittest.TestCase):
    def all_pass(self, p):
        ev, esp = ev_and_esp()
        return run.t1_checks(p, ev, esp)

    def test_green_probe_all_checks_pass(self):
        checks = self.all_pass(green_probe())
        failed = [n for n, c in checks.items() if not c["pass"]]
        self.assertEqual(failed, [], f"failed: {failed}")

    def test_brush_sh_fails_on_other_shell(self):
        checks = self.all_pass(green_probe(sh="/usr/bin/bash"))
        self.assertFalse(checks["brush_sh"]["pass"])

    def test_nushell_default_shell_requires_probe_ok(self):
        checks = self.all_pass(green_probe(nu_ok=0))
        self.assertFalse(checks["nushell_default_shell"]["pass"])

    def test_nushell_default_shell_fails_on_missing_field(self):
        p = green_probe()
        del p["nu_ok"]
        checks = self.all_pass(p)
        self.assertFalse(checks["nushell_default_shell"]["pass"])

    def test_admin_userland_fails_when_helix_missing(self):
        checks = self.all_pass(green_probe(helix_ok=0))
        self.assertFalse(checks["admin_userland"]["pass"])

    def test_admin_userland_fails_when_zellij_missing(self):
        checks = self.all_pass(green_probe(zellij_ok=0))
        self.assertFalse(checks["admin_userland"]["pass"])

    def test_admin_userland_fails_on_missing_fields(self):
        p = green_probe()
        del p["helix_ok"]
        del p["zellij_ok"]
        checks = self.all_pass(p)
        self.assertFalse(checks["admin_userland"]["pass"])


if __name__ == "__main__":
    unittest.main()
