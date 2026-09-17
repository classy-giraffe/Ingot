#!/usr/bin/env python3
"""Mirror a frozen Fedora Koji compose into a local, dnf-usable repository.

Mirrors the ``Everything/x86_64/os`` tree of a pinned rawhide compose:

* the original ``repodata/`` tree, unmodified (identical metadata keeps
  dnf resolution identical to a network build),
* every RPM with arch ``x86_64`` or ``noarch`` (i686 packages are never
  requested by an x86_64 build and are skipped),
* a ``manifest.json`` recording the compose ID, URL, and per-file
  sha256/size for verification and auditing.

The destination directory is resumable: files whose size and sha256
already match the compose metadata are skipped.
"""

import argparse
import concurrent.futures
import hashlib
import sys
import tempfile
import time
import urllib.request
import xml.etree.ElementTree as ET
import json
import subprocess
import zlib

REPO_NS = {"repo": "http://linux.duke.edu/metadata/repo"}
RPM_NS = {"rpm": "http://linux.duke.edu/metadata/common"}
KEEP_ARCHS = {"x86_64", "noarch"}


RETRIES = 8
BACKOFF = 2.0


def get(url: str, timeout: int = 300) -> bytes:
    """Fetch a URL, retrying transient CDN failures (502/503, DNS,
    timeouts) with exponential backoff. The rawhide CDN is flaky; a
    single 502 must not kill a 60k-file mirror run."""
    last_exc: Exception | None = None
    for attempt in range(1, RETRIES + 1):
        try:
            with urllib.request.urlopen(url, timeout=timeout) as r:
                return r.read()
        except Exception as e:  # URLError, HTTPError, socket.timeout, ...
            last_exc = e
            if attempt == RETRIES:
                break
            delay = min(BACKOFF * (2 ** (attempt - 1)), 60.0)
            print(f"  retry {attempt}/{RETRIES - 1} in {delay:.0f}s: {e}", file=sys.stderr)
            time.sleep(delay)
    raise RuntimeError(f"GET {url} failed after {RETRIES} attempts: {last_exc}")


def zstd_decompress(blob: bytes) -> bytes:
    with tempfile.NamedTemporaryFile(suffix=".zst") as tf:
        tf.write(blob)
        tf.flush()
        return subprocess.run(
            ["zstdcat", tf.name], capture_output=True, check=True
        ).stdout


def sha256_file(path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def repodata_files(base: str):
    repomd = ET.fromstring(get(f"{base}/repodata/repomd.xml"))
    files = []
    for data in repomd.findall("repo:data", REPO_NS):
        for loc in data.findall("repo:location", REPO_NS):
            href = loc.get("href")
            files.append(href)
    return files


def parse_primary(base: str):
    """Return (repodata_files, packages) from the compose."""
    repomd = ET.fromstring(get(f"{base}/repodata/repomd.xml"))
    files = []
    primary_href = None
    for data in repomd.findall("repo:data", REPO_NS):
        for loc in data.findall("repo:location", REPO_NS):
            files.append(loc.get("href"))
            if data.get("type") == "primary":
                primary_href = loc.get("href")
    if primary_href is None:
        sys.exit("no primary metadata found in repomd")

    blob = get(f"{base}/{primary_href}")
    if primary_href.endswith(".zst"):
        xml_bytes = zstd_decompress(blob)
    elif primary_href.endswith(".gz"):
        xml_bytes = zlib.decompress(blob, 47)
    elif primary_href.endswith(".zck"):
        sys.exit("primary metadata is zck only; unexpected for a compose")
    else:
        xml_bytes = blob

    root = ET.fromstring(xml_bytes)
    packages = []
    for p in root.findall("rpm:package", RPM_NS):
        arch = p.find("rpm:arch", RPM_NS)
        if arch is None or arch.text not in KEEP_ARCHS:
            continue
        loc = p.find("rpm:location", RPM_NS)
        checksum = p.find("rpm:checksum", RPM_NS)
        size = p.find("rpm:size", RPM_NS)
        packages.append(
            {
                "href": loc.get("href"),
                "sha256": checksum.get("value") if checksum is not None else None,
                "size": int(size.get("package", 0)),
            }
        )
    return files, packages


def download_file(base: str, href: str, destdir, expected_sha: str | None, expected_size: int | None) -> None:
    import os

    dest = destdir / href
    if dest.exists():
        size_ok = expected_size is None or dest.stat().st_size == expected_size
        if size_ok and (expected_sha is None or sha256_file(dest) == expected_sha):
            return
        dest.unlink()
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    blob = get(f"{base}/{href}")
    if expected_size is not None and len(blob) != expected_size:
        raise RuntimeError(f"{href}: size {len(blob)} != expected {expected_size}")
    if expected_sha is not None and hashlib.sha256(blob).hexdigest() != expected_sha:
        raise RuntimeError(f"{href}: sha256 mismatch")
    tmp.write_bytes(blob)
    os.replace(tmp, dest)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--compose-id", required=True)
    ap.add_argument("--base-url", default="https://kojipkgs.fedoraproject.org/compose/rawhide")
    ap.add_argument("--destdir", required=True, help="target directory (repo root goes into <destdir>/compose/Everything/x86_64/os)")
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--repodata-only", action="store_true", help="mirror repodata only (for inspection)")
    args = ap.parse_args()

    from pathlib import Path

    destdir = Path(args.destdir) / "compose" / "Everything" / "x86_64" / "os"
    base = f"{args.base_url}/{args.compose_id}/compose/Everything/x86_64/os"

    print(f"compose: {args.compose_id}", file=sys.stderr)
    print(f"base:    {base}", file=sys.stderr)
    print(f"dest:    {destdir}", file=sys.stderr)

    repodata_files, packages = parse_primary(base)
    total_bytes = sum(p["size"] for p in packages)
    print(
        f"repodata files: {len(repodata_files)}, packages: {len(packages)}, "
        f"total: {total_bytes / 1e9:.2f} GB",
        file=sys.stderr,
    )
    jobs = [("repodata/repomd.xml", None, None)]
    jobs += [(f, None, None) for f in repodata_files]
    if not args.repodata_only:
        jobs += [(p["href"], p["sha256"], p["size"]) for p in packages]

    done = 0
    def work(job):
        href, sha, size = job
        download_file(base, href, destdir, sha, size)

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futs = {pool.submit(work, j): j[0] for j in jobs}
        for fut in concurrent.futures.as_completed(futs):
            fut.result()
            done += 1
            if done % 500 == 0:
                print(f"  {done}/{len(jobs)}", file=sys.stderr)

    print("downloading complete; building manifest", file=sys.stderr)
    manifest = {
        "compose_id": args.compose_id,
        "url": f"{args.base_url}/{args.compose_id}",
        "tree": "compose/Everything/x86_64/os",
        "files": [],
    }
    for job in jobs:
        href = job[0]
        path = destdir / href
        manifest["files"].append(
            {"path": href, "sha256": sha256_file(path), "size": path.stat().st_size}
        )
    manifest["file_count"] = len(manifest["files"])
    manifest["total_bytes"] = sum(f["size"] for f in manifest["files"])
    out = destdir.parent.parent.parent / "manifest.json"
    out.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"manifest: {out} ({manifest['file_count']} files)", file=sys.stderr)


if __name__ == "__main__":
    main()
