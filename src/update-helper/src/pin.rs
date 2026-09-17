//! The pin document: the helper's output.
//!
//! Shape: the version, the tag, the per-tag asset base URL, and the
//! per-asset URLs + sha256 from the authenticated `SHA256SUMS`. This
//! is the shape the sysupdate transfers consume (research,
//! docs/research/sysupdate-transfer.md): `asset_base` is the
//! `[Source] Path=` value, each asset `name` is the `MatchPattern=`
//! expansion, and `version` is the argument to
//! `systemd-sysupdate update <version>`.

use serde::Serialize;

use crate::sums::SumsEntry;
use crate::version::Version;

/// Pin document schema version.
pub const PIN_SCHEMA: u64 = 1;

#[derive(Debug, Serialize)]
pub struct Pin {
    pub schema: u64,
    /// The release source (`owner/repo`).
    pub repo: String,
    /// The selected release tag, e.g. `v0.3.0`.
    pub tag: String,
    /// The version without the tag's `v`, e.g. `0.3.0`.
    pub version: String,
    /// The per-tag asset base URL: `<asset-base>/<tag>`. For the
    /// production source this is
    /// `https://github.com/<owner>/<repo>/releases/download/<tag>`.
    pub asset_base: String,
    /// The release assets, in `SHA256SUMS` order.
    pub assets: Vec<PinAsset>,
}

#[derive(Debug, Serialize)]
pub struct PinAsset {
    /// The asset name (as in `SHA256SUMS`).
    pub name: String,
    /// The sha256 from the authenticated `SHA256SUMS`.
    pub sha256: String,
    /// The per-tag download URL: `<asset_base>/<name>`.
    pub url: String,
}

/// Builds the pin document for the selected release.
pub fn build(
    repo: &str,
    tag: &str,
    version: &Version,
    asset_base: &str,
    sums: &[SumsEntry],
) -> Pin {
    let asset_base = format!("{asset_base}/{tag}");
    let assets = sums
        .iter()
        .map(|s| PinAsset {
            name: s.name.clone(),
            sha256: s.sha256.clone(),
            url: format!("{asset_base}/{}", s.name),
        })
        .collect();
    Pin {
        schema: PIN_SCHEMA,
        repo: repo.to_string(),
        tag: tag.to_string(),
        version: version.to_string(),
        asset_base,
        assets,
    }
}
