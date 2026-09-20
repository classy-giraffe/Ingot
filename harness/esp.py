"""esp.py - ESP partition operations for the Ingot harness (host-side).

The deployed disk image is a GPT disk whose ESP (partition label ``esp``)
systemd-boot reads at boot. mtools cannot open a partitioned image
(offset forms unsupported in 4.0.49), so the ESP region is carved out
into a standalone FAT image for mtools operations and written back.

All operations here are on disk images (host-side, pre-boot or
post-boot); nothing in this module runs in the guest.
"""

import os
import subprocess
import tempfile
from pathlib import Path

import bootstate
import gpt

try:
    from virt.firmware.varstore import autodetect as _fw_autodetect
except ImportError:
    _fw_autodetect = None
ESP_LABEL = "esp"
MTOOLS_ENV = {**os.environ, "MTOOLS_SKIP_CHECK": "1"}


class EspError(Exception):
    pass


def _partition(disk, label=ESP_LABEL):
    g = gpt.read_gpt(disk)
    try:
        return g.by_label(label)
    except gpt.GptError as e:
        raise EspError(f"{disk}: {e}") from e


def carve_esp(disk, out, label=ESP_LABEL) -> Path:
    """Copy the ESP partition's contents out of *disk* to *out*."""
    e = _partition(disk, label)
    out = Path(out)
    with open(disk, "rb") as f:
        f.seek(e.first_lba * gpt.SECTOR)
        data = f.read(e.size_bytes)
    if len(data) != e.size_bytes:
        raise EspError(f"short read carving {label} from {disk}")
    out.write_bytes(data)
    return out


def restore_esp(disk, img, label=ESP_LABEL) -> None:
    """Write the carved FAT image *img* back over the ESP partition."""
    e = _partition(disk, label)
    data = Path(img).read_bytes()
    if len(data) != e.size_bytes:
        raise EspError(
            f"ESP image size {len(data)} != partition size {e.size_bytes}; refusing")
    with open(disk, "r+b") as f:
        f.seek(e.first_lba * gpt.SECTOR)
        f.write(data)
        f.flush()
        os.fsync(f.fileno())


def mtools(argv, img) -> str:
    """Run an mtools binary against the carved FAT image *img*."""
    r = subprocess.run(
        [argv[0], "-i", str(img), *argv[1:]],
        capture_output=True, text=True, env=MTOOLS_ENV,
        # never inherit the caller's stdin or controlling TTY: mtools
        # answers prompts interactively, opening /dev/tty itself when it
        # needs input (a pre-existing mcopy destination triggers an
        # overwrite prompt); with stdin=DEVNULL and no controlling
        # terminal (start_new_session) the prompt cannot be shown and
        # mtools stays non-interactive
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    if r.returncode != 0:
        raise EspError(f"{argv[0]} failed ({r.returncode}): {r.stderr.strip()}")
    return r.stdout


def esp_list(img):
    """All paths (absolute ``::/...``) on the ESP; directories end with ``/``."""
    out = mtools(["mdir", "-b", "-/"], img)
    return [l.strip() for l in out.splitlines() if l.strip().startswith("::/")]


def esp_files(img):
    """The UKI entry files (``EFI/Linux/*.efi``) as ``{entry_id: path}``.

    The entry ID is the filename without ``.efi`` (systemd-boot's entry
    identity; boot counters live in the name as ``+<tries>-<done>``).
    """
    out = {}
    for p in esp_list(img):
        if p.startswith("::/EFI/Linux/") and p.endswith(".efi"):
            out[p[len("::/EFI/Linux/"):].removesuffix(".efi")] = p
    return out


def esp_read(img, path: str) -> bytes:
    """Read an ESP file (``::/...`` or bare relative path).

    mcat dumps the raw device in mtools, so file reads go through
    mcopy (FAT path -> host temp file).
    """
    if not path.startswith("::/"):
        path = "::/" + path.lstrip("/")
    with tempfile.NamedTemporaryFile(delete=False) as t:
        tmp = t.name
    try:
        mtools(["mcopy", path, tmp], img)
        return Path(tmp).read_bytes()
    finally:
        os.unlink(tmp)


def esp_write(img, path: str, data: bytes) -> None:
    """Write *data* to an ESP path (parent directories must exist)."""
    if not path.startswith("::/"):
        path = "::/" + path.lstrip("/")
    with tempfile.NamedTemporaryFile(delete=False) as t:
        t.write(data)
        tmp = t.name
    try:
        mtools(["mcopy", "-B", tmp, path], img)
    finally:
        os.unlink(tmp)


def esp_rename(img, old: str, new: str) -> None:
    """Rename an ESP file (the boot-counter re-arm operation)."""
    if not old.startswith("::/"):
        old = "::/" + old.lstrip("/")
    if not new.startswith("::/"):
        new = "::/" + new.lstrip("/")
    mtools(["mren", old, new], img)


def esp_loader_conf(img) -> dict:
    try:
        return bootstate.parse_loader_conf(esp_read(img, "loader/loader.conf").decode())
    except (EspError, UnicodeDecodeError):
        return {}


def esp_forensics(img):
    """Machine-readable ESP state: entries, bless/counter state, versions,
    loader.conf, and the modeled default entry."""
    files = esp_files(img)
    entries = sorted(files)
    versions = {}
    for eid in entries:
        v = bootstate.osrel_version(esp_read(img, files[eid]))
        if v is not None:
            versions[bootstate.base_id(eid)] = v.strip('"')
    conf = esp_loader_conf(img)
    return {
        "entries": entries,
        "states": {e: bootstate.classify(e) for e in entries},
        "counters": {e: list(bootstate.parse_counters(e)) for e in entries},
        "versions": versions,
        "loader_conf": conf,
        "default": bootstate.default_entry(entries),
    }


def read_nvram_vars(vars_fd: Path | str) -> dict[str, str]:
    """Read UEFI variables from an OVMF variable store (.fd file).

    Returns a dict mapping variable name to a string summary of its value.
    Returns empty dict if virt-firmware is unavailable or file is missing.
    """
    p = Path(vars_fd)
    if _fw_autodetect is None or not p.exists():
        return {}
    try:
        vs = _fw_autodetect.open_varstore(str(p))
        if vs is None:
            return {}
        vl = vs.get_varlist()
        return {name: str(var) for name, var in vl.items()}
    except Exception:
        return {}
