//! ingot-update-helper
//!
//! The narrow Ingot update helper (spec 22): given a release source,
//! discover the latest release (highest semver non-prerelease tag from
//! the GitHub Releases API - the date-based latest is not used),
//! authenticate it (the manifest.json and SHA256SUMS GPG signatures
//! against the project keyring; SHA256SUMS cross-checked against the
//! manifest), and emit the pin document: the version plus the per-tag,
//! per-asset URLs, in the shape the systemd-sysupdate transfers
//! consume.
//!
//! The helper does not download or install anything: payload
//! transfer, integrity checks at install time, and slot management
//! remain native systemd-sysupdate behavior.

mod cli;
mod discovery;
mod fetch;
mod gpg;
mod manifest;
mod pin;
mod sums;
mod version;

use anyhow::Context;
use cli::Args;
use discovery::{fetch_releases, select_latest};
use fetch::fetcher_for;
use gpg::Keyring;
use manifest::{cross_check, validate, Manifest};
use sums::SumsEntry;

/// Runs the helper: discover, authenticate, build the pin.
fn run(args: Args) -> anyhow::Result<pin::Pin> {
    let keyring = Keyring::load(&args.keyring)?;

    // --- discovery: latest = highest semver non-prerelease tag ----
    let fetch = fetcher_for(&args.api);
    let releases = fetch_releases(fetch.as_ref(), &args.api)?;
    if releases.is_empty() {
        anyhow::bail!("no releases found for {}", args.repo);
    }
    let Some((tag, version)) = select_latest(&releases) else {
        anyhow::bail!(
            "no eligible releases found for {}: no non-draft release \
            has a vMAJOR.MINOR.PATCH tag (drafts and prereleases are excluded)",
            args.repo
        );
    };

    // --- authentication: manifest + SHA256SUMS -------------------
    // Both documents are independently signed; the key must be
    // present in the keyring and the signature valid, and the two
    // documents must agree on every asset.
    let fetch = fetcher_for(&args.asset_base);
    let tag_base = format!("{}/{}", args.asset_base, tag);
    let manifest_bytes =
        verify_signed(fetch.as_ref(), &keyring, &tag_base, "manifest.json")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("parsing manifest.json")?;
    validate(&manifest, &tag, &version)?;

    let sums_bytes = verify_signed(fetch.as_ref(), &keyring, &tag_base, "SHA256SUMS")?;
    let sums: Vec<SumsEntry> = sums::parse(&sums_bytes)?;
    cross_check(&manifest, &sums)?;

    Ok(pin::build(&args.repo, &tag, &version, &args.asset_base, &sums))
}

/// Fetches one document, mapping fetch failures to a clear error
/// that names the URL.
fn fetch_document(fetch: &dyn fetch::Fetch, url: &str) -> anyhow::Result<Vec<u8>> {
    fetch
        .get(url)
        .with_context(|| format!("fetching {url}"))
        .map(|r| r.body)
}

/// Fetches a document and its detached signature, and verifies the
/// signature against the keyring. `what` names the document in the
/// error text; the document bytes are returned.
fn verify_signed(
    fetch: &dyn fetch::Fetch,
    keyring: &Keyring,
    base: &str,
    what: &str,
) -> anyhow::Result<Vec<u8>> {
    let data = fetch_document(fetch, &format!("{base}/{what}"))?;
    let sig = fetch_document(fetch, &format!("{base}/{what}.gpg"))?;
    keyring
        .verify(&sig, &data)
        .map_err(|e| format!("{what}: {e}"))
        .map_err(anyhow::Error::msg)?;
    Ok(data)
}

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse(&argv) {
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("\n{}", cli::USAGE);
            return std::process::ExitCode::from(2);
        }
        Ok(args) => args,
    };
    let out = args.out.clone();
    match run(args) {
        Ok(pin) => {
            let json = serde_json::to_string_pretty(&pin).unwrap();
            match out {
                Some(path) => {
                    if let Err(e) = std::fs::write(&path, &format!("{json}\n")) {
                        eprintln!("error: cannot write {path}: {e}");
                        return std::process::ExitCode::from(1);
                    }
                }
                None => println!("{json}"),
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::ExitCode::from(1)
        }
    }
}
