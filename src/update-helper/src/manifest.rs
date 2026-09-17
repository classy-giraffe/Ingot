//! The release manifest (`manifest.json`, v1 schema) and the
//! coherence cross-check between the manifest and `SHA256SUMS`.
//!
//! The manifest is the release descriptor the helper authenticates
//! (spec 20: schema, version/tag, image version, build metadata,
//! asset array). `systemd-sysupdate` never parses it; it is the
//! helper's trust input. The manifest's asset array describes the
//! whole release; `SHA256SUMS` lists the transfer payload assets, so
//! coherence means: every `SHA256SUMS` asset is described by the
//! manifest with a matching sha256.

use serde::Deserialize;

use crate::sums::{is_sha256_hex, SumsEntry};
use crate::version::Version;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema: u64,
    pub version: String,
    // The tag echo is not cross-checked (the version-vs-tag
    // coherence check is authoritative); parsed for schema
    // completeness.
    #[serde(default)]
    #[allow(dead_code)]
    pub tag: Option<String>,
    // Schema fields the helper does not consume (description, not
    // trust input); kept so the v1 schema is parsed completely.
    #[serde(default)]
    #[allow(dead_code)]
    pub image_version: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub build: Option<serde_json::Value>,
    pub assets: Vec<ManifestAsset>,
}

#[derive(Debug, Deserialize)]
pub struct ManifestAsset {
    pub name: String,
    pub sha256: String,
}

/// Validates the manifest against the selected release.
pub fn validate(m: &Manifest, tag: &str, version: &Version) -> anyhow::Result<()> {
    if m.schema != 1 {
        anyhow::bail!(
            "manifest.json: unsupported schema version {} (expected 1)",
            m.schema
        );
    }
    if m.version != version.to_string() {
        anyhow::bail!(
            "manifest.json: version '{}' does not match release tag '{tag}'",
            m.version
        );
    }
    if m.assets.is_empty() {
        anyhow::bail!("manifest.json: asset array is empty");
    }
    let mut seen = std::collections::HashSet::new();
    for a in &m.assets {
        if a.name.is_empty() || !is_sha256_hex(&a.sha256) {
            anyhow::bail!("manifest.json: malformed asset entry '{}'", a.name);
        }
        if !seen.insert(&a.name) {
            anyhow::bail!("manifest.json: duplicate asset '{}'", a.name);
        }
    }
    Ok(())
}

/// Cross-checks the authenticated `SHA256SUMS` against the
/// authenticated manifest: every sums asset must be listed in the
/// manifest with the same sha256. A mismatch means the two
/// independently signed documents disagree about the release -
/// incoherent, reject.
pub fn cross_check(m: &Manifest, sums: &[SumsEntry]) -> anyhow::Result<()> {
    for s in sums {
        match m.assets.iter().find(|a| a.name == s.name) {
            None => {
                anyhow::bail!(
                    "SHA256SUMS: asset '{}' is not listed in manifest.json",
                    s.name
                )
            }
            Some(a) if a.sha256 != s.sha256 => {
                anyhow::bail!(
                    "SHA256SUMS: asset '{}' has sha256 {} but manifest.json lists {}",
                    s.name,
                    s.sha256,
                    a.sha256
                )
            }
            Some(_) => {}
        }
    }
    Ok(())
}
