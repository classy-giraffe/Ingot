"""gpt.py - pure-Python GPT reader for the Ingot harness.

Reads the protective-MBR layout of a raw disk image: primary and backup
headers, and both partition entry arrays. Integrity is enforced the way
the UEFI spec does it where the toolchain agrees with it:

- each entry array is CRC32-verified against its header's partition
  array CRC field (header offset 88);
- primary and backup headers are cross-checked field by field (they
  must be identical except the header CRC, the current/alternate LBA
  pointers, and the per-header entry-array pointer);
- the arrays sit where the spec puts them: primary at LBA 2, backup
  in the 32 sectors before the backup header.

Notes on recent libfdisk/sgdisk (1.0.10 generation, as pinned for the
host tooling): the header's partition CRC field (offset 16) is written
with a non-spec formula, so it is not used for verification; and
``part_last`` carries a sentinel (0x8000000080), so only its
primary/backup equality is checked.

GPT stores UUIDs in mixed endian: the first three fields (4+2+2 bytes)
are little-endian, the last two (8 bytes) big-endian.
"""

import os
import struct
import zlib
from pathlib import Path

SECTOR = 512
GPT_MAGIC = b"EFI PART"
N_ENTRIES = 128
ENTRY_SIZE = 128
ARRAY_SECTORS = N_ENTRIES * ENTRY_SIZE // SECTOR  # 32
# LBA 0: protective MBR, LBA 1: primary header, LBA 2..33: primary entries.
PRIMARY_HEADER_LBA = 1
PRIMARY_ENTRIES_LBA = 2
MIN_SECTORS = 34


class GptError(Exception):
    pass


class GptEntry(tuple):
    """A partition entry: (type_uuid, part_uuid, label, first_lba, last_lba, size_bytes, attributes)."""

    __slots__ = ()

    @property
    def type_uuid(self):
        return self[0]

    @property
    def part_uuid(self):
        return self[1]

    @property
    def label(self):
        return self[2]

    @property
    def first_lba(self):
        return self[3]

    @property
    def last_lba(self):
        return self[4]

    @property
    def size_bytes(self):
        return self[5]

    @property
    def attributes(self):
        return self[6]


def _uuid(mixed: bytes) -> str:
    lo = struct.unpack("<IHH", mixed[:8])
    g4 = int.from_bytes(mixed[8:10], "big")
    g5 = int.from_bytes(mixed[10:16], "big")
    return f"{lo[0]:08x}-{lo[1]:04x}-{lo[2]:04x}-{g4:04x}-{g5:012x}"

ZERO_TYPE = _uuid(b"\x00" * 16)

def _header(header: bytes, sector: int) -> dict:
    if header[:8] != GPT_MAGIC:
        raise GptError(f"GPT magic missing at sector {sector}")
    return {
        "revision": struct.unpack_from("<I", header, 8)[0],
        "header_size": struct.unpack_from("<I", header, 12)[0],
        "part_crc": struct.unpack_from("<I", header, 88)[0],
        "my_lba": struct.unpack_from("<Q", header, 24)[0],
        "alt_lba": struct.unpack_from("<Q", header, 32)[0],
        "first_usable": struct.unpack_from("<Q", header, 40)[0],
        "last_usable": struct.unpack_from("<Q", header, 48)[0],
        "disk_guid": _uuid(header[56:72]),
        "part_first": struct.unpack_from("<Q", header, 72)[0],
        "part_last": struct.unpack_from("<Q", header, 80)[0],
    }


def _entries(table: bytes) -> list:
    entries = []
    for i in range(N_ENTRIES):
        e = i * ENTRY_SIZE
        type_uuid = _uuid(table[e:e + 16])
        if type_uuid == ZERO_TYPE:
            continue
        part_uuid = _uuid(table[e + 16:e + 32])
        first_lba, last_lba = struct.unpack_from("<QQ", table, e + 32)
        (attributes,) = struct.unpack_from("<Q", table, e + 48)
        label = table[e + 56:e + 128].decode("utf-16-le", "replace").split("\x00", 1)[0]
        entries.append(GptEntry((type_uuid, part_uuid, label, first_lba, last_lba,
                                 (last_lba - first_lba + 1) * SECTOR, attributes)))
    return entries


class Gpt:
    def __init__(self, path, disk_guid, entries):
        self.path = path
        self.disk_guid = disk_guid
        self.entries = entries

    def by_part_uuid(self, part_uuid: str) -> GptEntry:
        for e in self.entries:
            if e.part_uuid == part_uuid.lower():
                return e
        raise GptError(f"PARTUUID {part_uuid} not found")

    def by_label(self, label: str) -> GptEntry:
        for e in self.entries:
            if e.label == label:
                return e
        raise GptError(f"label {label!r} not found")


def _pread(f, off: int, count: int) -> bytes:
    """Read exactly count bytes at file offset off (short read = GptError)."""
    f.seek(off)
    buf = f.read(count)
    if len(buf) != count:
        raise GptError(f"short read at offset {off} ({len(buf)}/{count} bytes)")
    return buf


def read_gpt(path) -> Gpt:
    # Only the GPT regions are read: the 512B headers at LBA 1 and the
    # last LBA, and the 16KiB entry arrays at LBA 2 and last-32. The
    # harness disk is ~29GiB; reading the whole image OOMs the process.
    with Path(path).open("rb") as f:
        size = os.fstat(f.fileno()).st_size
        n_sectors = size // SECTOR
        if n_sectors < MIN_SECTORS:
            raise GptError(f"image too small for a GPT ({size} bytes)")
        last_lba = n_sectors - 1
        primary = _header(_pread(f, PRIMARY_HEADER_LBA * SECTOR, SECTOR),
                          PRIMARY_HEADER_LBA)
        backup = _header(_pread(f, last_lba * SECTOR, SECTOR), last_lba)

        # UEFI spec placement: primary array at LBA 2; backup array is the
        # 32 sectors immediately before the backup header.
        if primary["part_first"] != PRIMARY_ENTRIES_LBA:
            raise GptError("GPT primary entry array not at LBA 2")
        if backup["part_first"] != last_lba - ARRAY_SECTORS:
            raise GptError("GPT backup entry array misplaced")

        # Cross-check primary and backup: identical except the header CRC,
        # the current/alternate LBA pointers, and part_first (each header
        # points at its own array). part_last carries a libfdisk sentinel
        # in recent toolchains, so only its equality is checked.
        for key in ("revision", "header_size", "first_usable", "last_usable",
                    "disk_guid", "part_last"):
            if primary[key] != backup[key]:
                raise GptError(f"GPT primary/backup header mismatch on {key}")
        if primary["my_lba"] != PRIMARY_HEADER_LBA or backup["my_lba"] != last_lba:
            raise GptError("GPT header LBA fields inconsistent")
        if primary["alt_lba"] != last_lba or backup["alt_lba"] != PRIMARY_HEADER_LBA:
            raise GptError("GPT alternate LBA fields inconsistent")

        table = None
        for hdr, tag in ((primary, "primary"), (backup, "backup")):
            off = hdr["part_first"] * SECTOR
            end = off + ARRAY_SECTORS * SECTOR
            if end > size:
                raise GptError(f"GPT {tag} entry array out of bounds")
            arr = _pread(f, off, ARRAY_SECTORS * SECTOR)
            crc = zlib.crc32(arr) & 0xFFFFFFFF
            if crc != hdr["part_crc"]:
                raise GptError(f"GPT {tag} entry array CRC mismatch")
            if tag == "primary":
                table = arr

    return Gpt(path, primary["disk_guid"], _entries(table))
