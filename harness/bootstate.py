"""bootstate.py - machine-readable boot selection state.

Interprets the on-ESP artifacts systemd-boot uses for A/B selection:

- Entry names. A versioned entry (``ingot_0.2.0.efi``) with a
  ``+<tries-left>`` suffix is armed for boot attempts: the loader
  decrements the first number on each boot it offers; the second
  number (``+<tries-left>-<attempts>``) counts the attempts used.
  An entry with no counter is blessed (or never armed); an entry
  whose tries-left reached zero is bad.
- ``loader.conf`` (the ``default``/``timeout``/``auto-continue`` lines).
- Per-entry version from the UKI's ``.osrel`` section (VERSION_ID),
  so the state report carries the OS version, not just the entry ID.

``default_entry`` models systemd-boot's selection: bad entries
(tries-left exhausted) rank last; among the rest, the newest version
wins (versionsort). An armed-but-untried entry is selected like any
other - the counter is its protection, not exclusion.
"""
try:
    import pefile
except ImportError:
    pefile = None

import re
import struct

_CTR_RE = re.compile(r"^(?P<id>.+?)(?:\+(?P<left>\d+)(?:-(?P<attempts>\d+))?)?$")


def parse_counters(name: str):
    """(tries-left, attempts) from an entry name; (None, None) when unarmed."""
    base = name[:-4] if name.endswith(".efi") else name
    m = _CTR_RE.match(base)
    if not m or m.group("left") is None:
        return (None, None)
    return (int(m.group("left")), int(m.group("attempts") or 0))


def classify(name: str) -> str:
    """Bless state of an entry: good / indeterminate / bad.

    Bad once the loader's tries-left count reached zero.
    """
    tries_left, _attempts = parse_counters(name)
    if tries_left is None:
        return "good"
    if tries_left == 0:
        return "bad"
    return "indeterminate"


def base_id(name: str) -> str:
    """The entry ID with any ``.efi`` and counter suffix stripped."""
    base = name[:-4] if name.endswith(".efi") else name
    m = _CTR_RE.match(base)
    return m.group("id") if m else base


def _version_key(v: str):
    """systemd versionsort: numbers compare numerically, runs of
    non-digits lexically; a non-digit run sorts before a digit run."""
    key = []
    for p in re.findall(r"\d+|\D", v):
        if p.isdigit():
            key.append((1, int(p), ""))
        else:
            key.append((0, 0, p))
    return key


def default_entry(names) -> str | None:
    """The entry systemd-boot would select as default."""
    names = list(names)
    if not names:
        return None
    non_bad = [n for n in names if classify(n) != "bad"]
    pool = non_bad if non_bad else names
    pool.sort(key=lambda n: _version_key(base_id(n)), reverse=True)
    return pool[0]


def parse_loader_conf(text: str) -> dict:
    """Parse the systemd-boot loader.conf key/value lines."""
    conf = {}
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        parts = s.split(None, 1)
        if len(parts) == 1:
            conf[parts[0]] = ""
        else:
            conf[parts[0]] = parts[1].strip()
    return conf


def osrel_version(data: bytes):
    """VERSION_ID from a UKI's .osrel section; None when absent or unparseable."""
    if len(data) < 0x40:
        return None
    if pefile is not None:
        try:
            pe = pefile.PE(data=data, fast_load=True)
            for s in pe.sections:
                if s.Name.rstrip(b"\x00") == b".osrel":
                    text = s.get_data().decode("utf-8", "replace")
                    for line in text.splitlines():
                        if line.startswith("VERSION_ID="):
                            return line.split("=", 1)[1].strip()
            return None
        except Exception:
            pass
    try:
        (e_lfanew,) = struct.unpack_from("<I", data, 0x3C)
        off = e_lfanew
        if data[off:off + 4] != b"PE\x00\x00":
            return None
        coff = off + 4
        (nsec,) = struct.unpack_from("<H", data, coff + 2)
        (size_opt,) = struct.unpack_from("<H", data, coff + 16)
        opt = coff + 20
        (magic,) = struct.unpack_from("<H", data, opt)
        if magic != 0x20B:
            return None
        sect = opt + size_opt
        end = sect + nsec * 40
        if end > len(data):
            return None
        for i in range(nsec):
            e = sect + i * 40
            name = data[e:e + 8]
            if not name.startswith(b".osrel"):
                continue
            raw_size, raw_ptr = struct.unpack_from("<II", data, e + 16)
            if raw_ptr + raw_size > len(data):
                return None
            text = data[raw_ptr:raw_ptr + raw_size].decode("utf-8", "replace")
            for line in text.splitlines():
                if line.startswith("VERSION_ID="):
                    return line.split("=", 1)[1].strip()
    except (struct.error, IndexError):
        return None
    return None


def selection_state(entries, loader_conf, versions=None) -> dict:
    """Assemble the machine-readable boot selection state.

    entries: entry IDs on the ESP (with or without .efi, counter suffix
    intact). loader_conf: parsed loader.conf. versions: optional
    base-ID -> VERSION_ID map from the UKIs' .osrel sections.
    """
    versions = versions or {}
    out = []
    for name in entries:
        tries_left, attempts = parse_counters(name)
        state = classify(name)
        out.append({
            "id": name,
            "state": state,
            "blessed": state == "good",
            "tries": tries_left,
            "tries_left": tries_left,
            "attempts": attempts if tries_left is not None else 0,
            "version": versions.get(base_id(name)),
        })
    return {
        "entries": out,
        "default": default_entry(entries),
        "loader_conf": dict(loader_conf),
    }
