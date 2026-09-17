//! Release discovery: fetch the GitHub Releases list and select the
//! latest eligible release.
//!
//! "Latest" is the highest semver non-prerelease tag (spec 22: the
//! date-based latest is not used). The Releases API returns releases
//! newest-first by date; the selection here is purely by version.

use anyhow::Context;
use serde::Deserialize;

use crate::fetch::Fetch;
use crate::version::{parse_tag, Version};

/// The fields of a GitHub release the helper consumes. The API
/// response carries more; serde ignores the rest.
#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
}

/// Fetches the full release list, following `Link: rel="next"`
/// pagination. The first page is requested with `per_page=100` (the
/// API maximum) for http(s) URLs; `file://` fixtures are single-page.
pub fn fetch_releases(fetch: &dyn Fetch, api_url: &str) -> anyhow::Result<Vec<Release>> {
    let mut all = Vec::new();
    let mut url = if is_http(api_url) {
        let sep = if api_url.contains('?') { "&" } else { "?" };
        format!("{api_url}{sep}per_page=100")
    } else {
        api_url.to_string()
    };
    loop {
        let resp = fetch
            .get(&url)
            .with_context(|| format!("fetching release list {url}"))?;
        let page: Vec<Release> = serde_json::from_slice(&resp.body)
            .with_context(|| format!("parsing release list from {url}"))?;
        let n = page.len();
        all.extend(page);
        match resp.next {
            Some(next) if n > 0 => url = next,
            _ => break,
        }
    }
    Ok(all)
}

fn is_http(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Selects the latest eligible release: the highest semver tag among
/// non-draft, non-prerelease releases whose tag is a plain
/// `v?MAJOR.MINOR.PATCH`. Returns the selected tag and its parsed
/// version, or `None` when no release is eligible.
///
/// Creation dates are deliberately ignored: a backported or delayed
/// release with the highest version wins, regardless of when it was
/// published.
pub fn select_latest(releases: &[Release]) -> Option<(String, Version)> {
    let mut best: Option<(String, Version)> = None;
    for r in releases {
        if r.draft || r.prerelease {
            continue;
        }
        let Some(v) = parse_tag(&r.tag_name) else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, bv)| v > *bv) {
            best = Some((r.tag_name.clone(), v));
        }
    }
    best
}
