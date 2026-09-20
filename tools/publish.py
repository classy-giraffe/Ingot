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
    if not checks:
        raise RuntimeError(
            f"publish: harness results at '{results_path}' contain no check entries - "
            f"cannot publish without a green harness result"
        )
    failed_checks = []
    for name, check_item in checks.items():
        if isinstance(check_item, dict):
            ok = check_item.get("ok", check_item.get("pass", False))
        else:
            ok = bool(check_item)
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

    stderr_lower = res.stderr.lower()
    if "not found" not in stderr_lower and "no release" not in stderr_lower:
        raise RuntimeError(
            f"publish: failed to verify release immutability on {repo}: {res.stderr.strip()}"
        )

    return True


def verify_release_assets(
    dist_dir: Path, version: str, pubkey_path: Path
) -> list[Path]:
    """Verify all release assets exist and detached GPG signatures are valid."""
    required_names = release.canonical_asset_names(version)
    required_files = [dist_dir / name for name in required_names]

    for asset_file in required_files:
        if not asset_file.exists():
            raise FileNotFoundError(
                f"publish: required release asset missing: {asset_file}"
            )

    signatures = [f for f in required_files if f.name.endswith(".gpg")]
    release.verify_signatures_set(dist_dir, signatures, pubkey_path)

    return required_files


def validate_publish_gates(
    repo: str,
    tag: str,
    version: str,
    dist_dir: Path,
    harness_results: Path,
    pubkey_path: Path,
) -> list[Path]:
    """Validate all three publish gates before uploading."""
    print(f"publish: validating gates for {repo} {tag}...")

    check_harness_gate(harness_results)
    print(f"publish: [Gate 1] harness results green at {harness_results} -> PASS")

    check_release_immutability(repo, tag)
    print(
        f"publish: [Gate 2] release {tag} does not exist on {repo} (immutable) -> PASS"
    )

    upload_files = verify_release_assets(dist_dir, version, pubkey_path)
    print(
        f"publish: [Gate 3] all {len(upload_files)} assets and GPG signatures verified -> PASS"
    )
    return upload_files


def dispatch_github_release(
    repo: str,
    tag: str,
    upload_files: list[Path],
    notes: str,
    prerelease: bool = False,
) -> str:
    """Invoke gh release create to publish the release and upload assets."""
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
    return release_url


def default_release_notes(version: str, compose_id: str) -> str:
    """Construct default release notes describing the release and compose pin."""
    return (
        f"Ingot {version} release.\n\n"
        f"Base compose: {compose_id}\n"
        "All release assets are signed with the project GPG key and verified against the vendor keyring."
    )


def resolve_publish_params(
    version: str | None,
    dist_dir: Path | None,
    harness_results: Path | None,
    repo_root: Path,
) -> tuple[str, Path, Path, str]:
    """Resolve version, paths, and compose ID from repository pins."""
    pins = json.loads((repo_root / "tools/pins.json").read_text())
    resolved_version = version or pins["image_version"]
    dist = dist_dir or (repo_root / "dist")
    harness = harness_results or (dist / "harness/results.json")
    return resolved_version, dist, harness, pins["compose"]["id"]


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
    version, dist_dir, harness_results, compose_id = resolve_publish_params(
        version, dist_dir, harness_results, repo_root
    )
    tag = f"v{version}"
    pubkey_path = repo_root / "tools/keys/project.pgp"

    upload_files = validate_publish_gates(
        repo=repo,
        tag=tag,
        version=version,
        dist_dir=dist_dir,
        harness_results=harness_results,
        pubkey_path=pubkey_path,
    )

    if notes is None:
        notes = default_release_notes(version, compose_id)

    asset_names = [f.name for f in upload_files]
    if dry_run:
        print(
            f"publish: dry-run mode - would publish {tag} with {len(upload_files)} assets to {repo}"
        )
        return {"status": "dry_run", "repo": repo, "tag": tag, "assets": asset_names}

    url = dispatch_github_release(repo, tag, upload_files, notes, prerelease)
    return {
        "status": "published",
        "repo": repo,
        "tag": tag,
        "url": url,
        "assets": asset_names,
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
