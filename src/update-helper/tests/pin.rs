//! End-to-end tests of the ingot-update-helper binary against the
//! committed fixtures (tests/fixtures/). Everything runs offline: the
//! releases list and asset documents are read through file:// URLs,
//! and the GPG signatures are real gpg-generated detached
//! signatures verified by sequoia-openpgp against the fixture
//! keyring. External behavior only: process exit codes, stdout
//! (the pin document), stderr (the error text), and written files.
//!
//! The fixture set (see tests/fixtures/keys/README.md):
//! - basic: three stable releases; v0.3.0 is latest.
//! - prerelease: a flagged prerelease and an unflagged
//!   prerelease-suffixed tag are newer by date; v0.2.1 is latest.
//! - multidigit: v1.10.1 beats v1.9.0 numerically.
//! - datetrap: v0.9.0 is version-highest; the date-newest release
//!   (v0.2.0) is a backport.
//! - empty / prerelease-only: no eligible stable release.
//! - tampered: manifest.json byte-flipped after signing.
//! - unknownkey: manifest signed by a key not in the keyring.
//! - tampered-sums: SHA256SUMS byte-flipped after signing.
//! - incoherent: signed SUMS and manifest disagree on a hash.
//! - sums-missing: signed SUMS lists an asset the manifest lacks.
//! - missing-manifest: the tag directory has no manifest.json.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn helper() -> &'static str {
    env!("CARGO_BIN_EXE_ingot-update-helper")
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs the helper with a repo fixture: releases list and asset base
/// are file:// URLs into tests/fixtures/repos/<repo>/, and the keyring
/// is the fixture project keyring.
fn run_repo(repo: &str, extra: &[&str]) -> Outcome {
    let f = fixtures();
    let api = format!("file://{}/repos/{repo}/releases.json", f.display());
    let asset_base = format!("file://{}/repos/{repo}/assets", f.display());
    let mut args = vec![
        "--repo".into(),
        "classy-giraffe/Ingot".into(),
        "--keyring".into(),
        format!("{}/keys/project.pgp", f.display()),
        "--api".into(),
        api,
        "--asset-base".into(),
        asset_base,
    ];
    for a in extra {
        args.push(a.to_string());
    }
    let out = Command::new(helper())
        .args(&args)
        .output()
        .expect("running ingot-update-helper");
    Outcome {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The expected sha256 for `name` from the fixture's own SHA256SUMS
/// (the document the pin is cross-checked against).
fn sums_hash(repo: &str, tag: &str, name: &str) -> String {
    let path = fixtures().join(format!("repos/{repo}/assets/{tag}/SHA256SUMS"));
    let text = std::fs::read_to_string(&path).expect("reading SHA256SUMS");
    for line in text.lines() {
        let rest = &line[64..];
        let entry_name = rest[2..].to_string();
        if entry_name == name {
            return line[..64].to_string();
        }
    }
    panic!("asset {name} not in {}", path.display());
}

#[test]
fn picks_highest_stable_version() {
    let o = run_repo("basic", &[]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    let pin: serde_json::Value =
        serde_json::from_str(&o.stdout).expect("pin is valid JSON");
    assert_eq!(pin["version"], "0.3.0");
    assert_eq!(pin["tag"], "v0.3.0");
}

#[test]
fn excludes_prereleases_flagged_and_suffixed() {
    let o = run_repo("prerelease", &[]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    let pin: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(pin["version"], "0.2.1");
    assert_eq!(pin["tag"], "v0.2.1");
}

#[test]
fn orders_multidigit_components_numerically() {
    let o = run_repo("multidigit", &[]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    let pin: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(pin["version"], "1.10.1");
    assert_eq!(pin["tag"], "v1.10.1");
}

#[test]
fn ignores_date_based_latest() {
    let o = run_repo("datetrap", &[]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    let pin: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(pin["version"], "0.9.0");
    assert_eq!(pin["tag"], "v0.9.0");
}

#[test]
fn no_releases_is_an_error() {
    let o = run_repo("empty", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(
        o.stderr.contains("no releases found") || o.stderr.contains("no eligible releases"),
        "{}",
        o.stderr
    );
}
#[test]
fn prerelease_only_is_an_error() {
    let o = run_repo("prerelease-only", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(o.stderr.contains("no eligible releases"), "{}", o.stderr);
}

// ------------------------------------------------------- authentication ---

#[test]
fn tampered_manifest_is_rejected() {
    let o = run_repo("tampered", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(o.stderr.contains("manifest.json"), "{}", o.stderr);
    assert!(o.stderr.contains("signature"), "{}", o.stderr);
}

#[test]
fn unknown_signing_key_is_rejected() {
    let o = run_repo("unknownkey", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(o.stderr.contains("manifest.json"), "{}", o.stderr);
    assert!(o.stderr.contains("not in keyring"), "{}", o.stderr);
}

#[test]
fn tampered_sums_are_rejected() {
    let o = run_repo("tampered-sums", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(o.stderr.contains("SHA256SUMS"), "{}", o.stderr);
    assert!(o.stderr.contains("signature"), "{}", o.stderr);
}

#[test]
fn incoherent_documents_are_rejected() {
    let o = run_repo("incoherent", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(
        o.stderr.contains("but manifest.json lists"),
        "{}",
        o.stderr
    );
}

#[test]
fn sums_entry_missing_from_manifest_is_rejected() {
    let o = run_repo("sums-missing", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(
        o.stderr.contains("not listed in manifest.json"),
        "{}",
        o.stderr
    );
}

#[test]
fn missing_manifest_is_an_error() {
    let o = run_repo("missing-manifest", &[]);
    assert_eq!(o.code, 1, "stdout: {}", o.stdout);
    assert!(o.stderr.contains("manifest.json"), "{}", o.stderr);
}

// ------------------------------------------------------------- pin shape ---

#[test]
fn pin_contains_per_asset_urls_and_hashes() {
    let o = run_repo("basic", &[]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    let pin: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    let assets = pin["assets"].as_array().expect("assets array");
    assert_eq!(assets.len(), 2);
    let f = fixtures();
    for (a, name) in assets
        .iter()
        .zip(["ingot_0.3.0.root.erofs", "ingot_0.3.0.efi"])
    {
        assert_eq!(a["name"], name);
        let url = format!(
            "file://{}/repos/basic/assets/v0.3.0/{name}",
            f.display()
        );
        assert_eq!(a["url"], url);
        assert_eq!(a["sha256"], sums_hash("basic", "v0.3.0", name));
    }
}

// ------------------------------------------------------------------- cli ---

#[test]
fn missing_required_flags_print_usage() {
    let out = Command::new(helper()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("ingot-update-helper"));
}

#[test]
fn unknown_flag_is_an_error() {
    let out = Command::new(helper()).arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bogus"));
}

#[test]
fn out_flag_writes_the_pin_to_a_file() {
    // unique per process: the test is hermetic under concurrent runs
    let path = std::env::temp_dir().join(format!("ingot-pin-test-{}.json", std::process::id()));
    let o = run_repo("basic", &["--out", path.to_str().unwrap()]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(o.stdout.is_empty(), "stdout should be empty: {}", o.stdout);
    let written = std::fs::read_to_string(&path).expect("pin file written");
    let pin: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(pin["version"], "0.3.0");
    std::fs::remove_file(&path).ok();
}
