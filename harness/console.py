"""console.py - serial console evidence parsing for the Ingot harness.

Two evidence modes (see harness/probe/ for the guest-side probe):

- Probe mode: the injected harness probe prints a framed JSON document
  (``PROBE_BEGIN`` ... ``PROBE_END``) to the serial console after the
  system reaches multi-user. ``extract_probe_frame`` recovers that
  document from a raw console capture, which carries kernel log
  prefixes, ANSI decorations, and interleaved journal-forwarded lines.

- Baseline mode (``--no-probe``): the probe is absent from the image, so
  the harness asserts only host-visible console lines: the kernel's
  ``Command line:`` echo, the manager's ``Reached target`` /
  ``Finished`` status lines, and journal-forwarded service output
  (which appears only on failure in this image).
"""

import json
import re

PROBE_BEGIN = "=== INGOT-HARNESS-PROBE-BEGIN ==="
PROBE_END = "=== INGOT-HARNESS-PROBE-END ==="

# Terminal escape sequences: CSI (e.g. \x1b[0;32m), OSC (e.g.
# \x1b]104\x1b\\, set-xterm-title), 2-character DEC sequences the
# serial getty's shell injects on input (ESC 7 / ESC 8 cursor
# save/restore, \x1b= keypad mode), and a dangling ESC. Alternatives
# are ordered so the longest match at each position wins.
_ANSI_RE = re.compile(
    r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"
    r"|\x1b\[[0-9;:<>?]*[ -/]*[@-~]"
    r"|\x1b[\x30-\x7e]"
    r"|\x1b"
)
# Kernel log prefix: "[    6.055131] " plus the optional journal
# forwarder "sh[662]: " / "systemd[1]: " / "unit.service[123]: ".
_KLOG_RE = re.compile(r"^\[\s*\d+(?:\.\d+)?\]\s*(?:\S+?:\s*)?")
# Journal-forwarded line from a concrete unit: "[ 5.1] unit.service[pid]: msg".
_JOURNAL_UNIT_RE = re.compile(r"^\[\s*\d+(?:\.\d+)?\]\s+\S+\.service\[\d+\]:")
# A failed-unit status line, e.g. "[FAILED] Failed to start foo.service - bar".
_FAILED_UNIT_RE = re.compile(r"Failed to start (\S+)\s+-")
# A line of a pretty-printed (indent=2) JSON document: the bare
# outer braces, indented openers/closers, or "key"/value lines.
# Deliberately strict: a console status line such as
# "[  OK  ] Some status" is rejected even though it starts with "[".
_JSON_LINE_RE = re.compile(
    r"^(?:"
    r"\{|\}"                                   # outer braces
    r"|\s*[{\[]\s*,?\s*$"                      # indented opener/closer
    r"|\s*\"(?:[^\"\\]|\\.)*\"\s*:\s*.*\s*,?\s*$"  # "key": value
    r"|\s*\"(?:[^\"\\]|\\.)*\"\s*,?\s*$"       # bare "value" element
    r"|\s*\"(?:[^\"\\]|\\.)*\"\s*$"            # bare "value" (last)
    r"|\s*-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\s*,?\s*$"  # number
    r"|\s*(?:true|false|null)\s*,?\s*$"        # literal
    r"|\s*\{.*\}\s*,?\s*$"                     # inline object
    r"|\s*\[.*\]\s*,?\s*$"                     # inline array
    r")"
)
# Stray terminal control bytes (NUL, ETX, form feed, ...) the serial
# getty interleaves with payload lines; never part of JSON.
_CTRL_RE = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")


def _strip_ansi(line: str) -> str:
    return _ANSI_RE.sub("", line)


def _strip_line(line: str) -> str:
    """Strip terminal escapes, stray control bytes, and the
    kernel/journal log prefix."""
    s = _CTRL_RE.sub("", _strip_ansi(line))
    return _KLOG_RE.sub("", s, count=1).rstrip()


def extract_probe_frame(text: str):
    """Recover the framed probe JSON document from a console capture.

    Returns the parsed document, or None when the frame is absent,
    unterminated, or the recovered JSON does not parse.
    """
    lines = text.split("\n")
    begin = None
    for i, raw in enumerate(lines):
        if _strip_line(raw) == PROBE_BEGIN:
            begin = i
            break
    if begin is None:
        return None
    end = None
    for j in range(begin + 1, len(lines)):
        if _strip_line(lines[j]) == PROBE_END:
            end = j
            break
    if end is None:
        return None

    kept = []
    for raw in lines[begin + 1:end]:
        s = _strip_line(raw)
        if "\x1b" in s:
            # escape sequences the stripper cannot cover: not payload
            continue
        if not s or not _JSON_LINE_RE.match(s):
            # Interleaved kernel/journal noise inside the frame.
            continue
        kept.append(s)
    try:
        return json.loads("\n".join(kept))
    except json.JSONDecodeError:
        return None


def kernel_cmdline(text: str):
    """The kernel's command line from the ``Command line:`` console line."""
    for line in text.split("\n"):
        s = _strip_line(line)
        if s.startswith("Command line:"):
            return s[len("Command line:"):].strip()
    return None


def reached_target(text: str, target: str) -> bool:
    """True if the manager reported reaching *target*.

    ``Stopped target`` (post-powerdown) does not count.
    """
    needle = "Reached target " + target
    for line in text.split("\n"):
        if needle in _strip_line(line):
            return True
    return False


def secure_boot_enabled(text: str) -> bool:
    return any("Secure boot enabled" in _strip_line(l) for l in text.split("\n"))


def console_contains(text: str, needle: str) -> bool:
    """True if any console line carries *needle* after ANSI escape and
    kernel/journal log prefix stripping.

    The manager's status lines (``[FAILED] Failed to start <unit>``)
    wrap the unit ID in ANSI color escapes, so a raw substring match
    misses them; matching goes on the stripped form (a raw substring
    of a clean line is also a substring of its stripped form).
    """
    return any(needle in _strip_line(line) for line in text.split("\n"))


def runtime_root_prepared(text: str) -> bool:
    """Baseline-mode check for the runtime-root prep completing.

    ``ingot-root.service`` finishes after mounting /var, binding /etc,
    and mounting the active slot at /usr; its completion line is the
    host-visible approximation of those invariants.
    """
    return any("Finished ingot-root.service" in _strip_line(l) for l in text.split("\n"))


def failed_units(text: str):
    """Unit names from ``Failed to start <unit> -`` lines, first-seen order."""
    units = []
    for line in text.split("\n"):
        m = _FAILED_UNIT_RE.search(_strip_ansi(line))
        if m and m.group(1) not in units:
            units.append(m.group(1))
    return units


def failure_markers(text: str):
    """Console lines evidencing an early-boot (initramfs) failure.

    In this image nothing writes to the journal during a healthy boot;
    a journal-forwarded line from a unit (e.g. ``ingot-prepare: ...``)
    is a failure signal.
    """
    markers = []
    for line in text.split("\n"):
        s = _strip_ansi(line).rstrip()
        if _JOURNAL_UNIT_RE.match(s):
            markers.append(s)
    return markers
