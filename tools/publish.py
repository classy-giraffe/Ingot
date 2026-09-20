#!/usr/bin/env python3
"""Ingot release publisher (T7: Publish gate).

Publishes the release to GitHub Releases with strict gates:
1. Harness gate: refuses to run without a green harness result.
2. Immutability gate: refuses to run if the release already exists on GitHub
   (no yank, no replace).
3. Verification gate: verifies all assets and detached GPG signatures
   against the project public key before upload.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Import helpers from tools/release.py
sys.path.insert(0, str(REPO_ROOT / "tools"))
import release


def check_harness_gate(results_path: Path) -> bool:
    """Verify that the harness results document exists and all checks passed."""
    if not results_path.exists():
        raise RuntimeError(
            f"publish: missing harness results at '{results_path}' - "
            f"cannot publish without a green harness result"
        )

    try:
        data = json.loads(results_path.read_text())
    except (json.JSONDecodeError, OSError) as e:
        raise RuntimeError(
            f"publish: failed to parse harness results at '{results_path}': {e}"
        ) from e

    if not data.get("pass", False):
        raise RuntimeError(
            f"publish: harness gate failed - overall pass is False in '{results_path}'"
        )

    checks = data.get("checks", {})
    failed_checks = []
    for name, c in checks.items():
        # A check can be boolean or object with ok/pass
        if isinstance(c, dict):
            ok = c.get("ok", c.get("pass", False))
        else:
            ok = bool(c)
        if not ok:
            failed_checks.append(name)

    if failed_checks:
        raise RuntimeError(
            f"publish: harness check(s) failed in '{results_path}': {', '.join(failed_checks)} - "
            f"cannot publish without a green harness result"
        )

    return True


def check_release_immutability(repo: str, tag: str) -> bool:
    """Verify that the release does not already exist on GitHub.

    Releases are immutable once published: no yank, no replace.
    """
    res = subprocess.run(
        ["gh", "release", "view", tag, "--repo", repo],
        capture_output=True,
        text=True,
        check=False,
    )
    if res.returncode == 0:
        raise RuntimeError(
            f"publish: release {tag} already exists on {repo} - "
            f"published releases are immutable (no yank, no replace)"
        )

    return True


def verify_release_assets(
    dist_dir: Path, version: str, pubkey_path: Path
) -> list[Path]:
    """Verify all release assets exist and detached GPG signatures are valid."""
    required_files = [
        dist_dir / f"ingot_{version}.root.erofs",
        dist_dir / f"ingot_{version}.root.erofs.gpg",
        dist_dir / f"ingot_{version}.efi",
        dist_dir / f"ingot_{version}.efi.gpg",
        dist_dir / f"ingot_{version}.iso",
        dist_dir / f"ingot_{version}.iso.gpg",
        dist_dir / "SHA256SUMS",
        dist_dir / "SHA256SUMS.gpg",
        dist_dir / "manifest.json",
        dist_dir / "manifest.json.gpg",
    ]

    for f in required_files:
        if not f.exists():
            raise FileNotFoundError(f"publish: required release asset missing: {f}")

    # Verify GPG signatures
    for f in required_files:
        if f.name.endswith(".gpg"):
            target = f.with_name(f.name[:-4])
            if not release.verify_file_signature(target, f, pubkey_path):
                raise RuntimeError(
                    f"publish: signature verification failed for {f.name} against {pubkey_path}"
                )

    return required_files


def publish_release(
    repo: str = "classy-giraffe/Ingot",
    version: str | None = None,
    dist_dir: Path | None = None,
    harness_results: Path | None = None,
    repo_root: Path = REPO_ROOT,
    notes: str | None = None,
    dry_run: bool = False,
    prerelease: bool = False,
) -> dict:
    """Publish the release to GitHub Releases, enforcing all gates."""
    if dist_dir is None:
        dist_dir = repo_root / "dist"
    pins = json.loads((repo_root / "tools/pins.json").read_text())
    if version is None:
        version = pins["image_version"]

    tag = f"v{version}"
    pubkey_path = repo_root / "tools/keys/project.pgp"

    # Default harness results path
    if harness_results is None:
        harness_results = dist_dir / "harness/results.json"

    print(f"publish: validating gates for {repo} {tag}...")

    # Gate 1: Harness gate
    check_harness_gate(harness_results)
    print(f"publish: [Gate 1] harness results green at {harness_results} -> PASS")

    # Gate 2: Immutability gate
    check_release_immutability(repo, tag)
    print(
        f"publish: [Gate 2] release {tag} does not exist on {repo} (immutable) -> PASS"
    )

    # Gate 3: Release asset verification & GPG signatures
    upload_files = verify_release_assets(dist_dir, version, pubkey_path)
    print(
        f"publish: [Gate 3] all {len(upload_files)} assets and GPG signatures verified -> PASS"
    )

    if notes is None:
        notes = (
            f"Ingot {version} release.\n\n"
            f"Base compose: {pins['compose']['id']}\n"
            f"All release assets are signed with the project GPG key and verified against the vendor keyring."
        )

    if dry_run:
        print(
            f"publish: dry-run mode - would publish {tag} with {len(upload_files)} assets to {repo}"
        )
        return {
            "status": "dry_run",
            "repo": repo,
            "tag": tag,
            "assets": [f.name for f in upload_files],
        }

    # Execute publication via gh release create
    cmd = [
        "gh",
        "release",
        "create",
        tag,
        *[str(f) for f in upload_files],
        "--repo",
        repo,
        "--title",
        f"Ingot {tag}",
        "--notes",
        notes,
    ]
    if prerelease:
        cmd.append("--prerelease")

    print(f"publish: creating GitHub release {tag} on {repo}...")
    res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if res.returncode != 0:
        raise RuntimeError(f"publish: gh release create failed: {res.stderr}")

    release_url = res.stdout.strip()
    print(f"publish: successfully published release: {release_url}")
    return {
        "status": "published",
        "repo": repo,
        "tag": tag,
        "url": release_url,
        "assets": [f.name for f in upload_files],
    }


def main():
    parser = argparse.ArgumentParser(description="Ingot release publish gate")
    parser.add_argument(
        "--repo",
        default="classy-giraffe/Ingot",
        help="GitHub repo (default: classy-giraffe/Ingot)",
    )
    parser.add_argument(
        "--version", help="Release version (default: from tools/pins.json)"
    )
    parser.add_argument("--dist", type=Path, help="Dist directory (default: dist/)")
    parser.add_argument(
        "--harness",
        type=Path,
        help="Harness results path (default: dist/harness/results.json)",
    )
    parser.add_argument("--notes", help="Release notes")
    parser.add_argument(
        "--dry-run", action="store_true", help="Validate gates without publishing"
    )
    parser.add_argument(
        "--prerelease", action="store_true", help="Mark release as prerelease on GitHub"
    )
    args = parser.parse_args()

    try:
        publish_release(
            repo=args.repo,
            version=args.version,
            dist_dir=args.dist,
            harness_results=args.harness,
            notes=args.notes,
            dry_run=args.dry_run,
            prerelease=args.prerelease,
        )
    except (RuntimeError, ValueError, FileNotFoundError, OSError) as e:
        print(f"publish error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
