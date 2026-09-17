//! Ingot release version: the numeric semver triple behind a release tag.
//!
//! Release tags are `vMAJOR.MINOR.PATCH` (e.g. `v0.3.0`); the version is
//! the same triple without the `v` prefix. The version is what slot
//! labels (`ingot_<v>`), UKI names, and `systemd-sysupdate update <v>`
//! consume.

/// A release version: numeric semver triple, compared numerically per
/// component (so 1.10.0 > 1.9.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parses a release tag into a version.
///
/// Only tags of the exact form `v?MAJOR.MINOR.PATCH` are eligible
/// releases. Anything else - prerelease-suffixed tags (`v1.2.3-rc1`),
/// CI or other non-release tags - is ineligible, not an error.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = tag.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut nums = [0u64; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // no leading zeros (semver rule); bounded width guards u64 parse
        if part.len() > 1 && part.starts_with('0') {
            return None;
        }
        if part.len() > 18 {
            return None;
        }
        nums[i] = part.parse().ok()?;
    }
    Some(Version {
        major: nums[0],
        minor: nums[1],
        patch: nums[2],
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_tag, Version};

    #[test]
    fn parses_v_prefixed_and_bare_semver() {
        assert_eq!(
            parse_tag("v0.3.0"),
            Some(Version { major: 0, minor: 3, patch: 0 })
        );
        assert_eq!(
            parse_tag("1.10.1"),
            Some(Version { major: 1, minor: 10, patch: 1 })
        );
    }

    #[test]
    fn rejects_non_release_tags() {
        // prerelease-suffixed (flag-independent exclusion)
        assert_eq!(parse_tag("v1.2.3-rc1"), None);
        assert_eq!(parse_tag("v0.4.0-beta.2"), None);
        // CI / non-release tags
        assert_eq!(parse_tag("ci-20260901"), None);
        assert_eq!(parse_tag("v1.2"), None);
        assert_eq!(parse_tag("v1.2.3.4"), None);
        // leading zeros are not semver
        assert_eq!(parse_tag("v01.2.3"), None);
        assert_eq!(parse_tag(""), None);
    }

    #[test]
    fn orders_numerically_per_component() {
        let mk = |t: &str| parse_tag(t).unwrap();
        // multidigit: numeric, not lexicographic (1.10.1 > 1.9.0,
        // while "1.10.1" < "1.9.0" as strings)
        assert!(mk("v1.10.1") > mk("v1.9.0"));
        assert!(mk("v1.9.0") < mk("v1.10.0"));
        assert!(mk("v0.10.0") > mk("v0.9.9"));
        // componentwise: major beats minor beats patch
        assert!(mk("v2.0.0") > mk("v1.99.99"));
        assert!(mk("v1.2.0") > mk("v1.1.99"));
        assert!(mk("v1.0.2") > mk("v1.0.1"));
        // equality
        assert_eq!(mk("v1.0.0"), mk("1.0.0"));
    }
}
