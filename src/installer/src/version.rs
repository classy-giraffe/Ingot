//! Ingot release version: the numeric semver triple an install
//! config's `[source] version` carries.
//!
//! The version selects the release artifact set
//! (`ingot_<v>.slot.raw`, `ingot_<v>.efi`, `ingot_<v>.esp.raw`) and
//! becomes the active slot's GPT label (`ingot_<v>`) and the UKI
//! entry name on the ESP. Compared numerically per component so
//! 1.10.0 > 1.9.0.

use std::fmt;

/// A release version: numeric semver triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Default for Version {
    fn default() -> Self {
        Version {
            major: 0,
            minor: 0,
            patch: 0,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parses a version string (`MAJOR.MINOR.PATCH`, plain numeric
/// components, no `v` prefix, no prerelease suffix). Anything else
/// is a diagnostic, not an error to recover from: the config is the
/// authorization and a bad version must not install anything.
pub fn parse(s: &str) -> Result<Version, String> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "'{s}' is not a version (expected MAJOR.MINOR.PATCH)"
        ));
    }
    let mut nums = [0u64; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 9
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!(
                "'{s}' is not a version (expected MAJOR.MINOR.PATCH)"
            ));
        }
        nums[i] = part.parse().unwrap();
    }
    Ok(Version {
        major: nums[0],
        minor: nums[1],
        patch: nums[2],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semver_triples() {
        assert_eq!(
            parse("0.1.0").unwrap(),
            Version {
                major: 0,
                minor: 1,
                patch: 0
            }
        );
        assert_eq!(
            parse("10.20.30").unwrap(),
            Version {
                major: 10,
                minor: 20,
                patch: 30
            }
        );
    }

    #[test]
    fn rejects_non_versions() {
        for bad in [
            "",
            "0.1",
            "1.2.3.4",
            "v0.1.0",
            "0.1.0-rc1",
            "a.b.c",
            "01.2.3",
            "0.1.x",
        ] {
            assert!(parse(bad).is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn orders_numerically_per_component() {
        let a = parse("1.9.0").unwrap();
        let b = parse("1.10.0").unwrap();
        assert!(a < b);
    }
}
