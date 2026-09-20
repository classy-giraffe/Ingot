//! Disk size parsing: the size values the install config's
//! `[partitions]` table carries.
//!
//! A size is a byte count with an optional binary suffix: `512K`,
//! `1M`, `8G`, `2T` (suffix case-insensitive, each 1024-based) or a
//! bare byte count. This matches the `SizeMinBytes=`/`SizeMaxBytes=`
//! semantics the values are rendered into for systemd-repart (systemd
//! size suffixes are 1024-based).

use std::fmt;

/// A parsed size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(pub u64);

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Suffix multipliers (1024-based).
const SUFFIXES: [(char, u64); 4] = [
    ('K', 1 << 10),
    ('M', 1 << 20),
    ('G', 1 << 30),
    ('T', 1 << 40),
];

/// One mebibyte.
pub const MIB: u64 = 1 << 20;

/// Parses a size string into bytes.
///
/// Accepted: `12345` (bare bytes), `512K`, `1M`, `8G`, `2t`. The
/// numeric part must be plain ASCII decimal.
pub fn parse_size(s: &str) -> Result<ByteSize, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size".into());
    }
    let (num, suffix) = match s.chars().next_back() {
        Some(c) if c.is_ascii_alphabetic() => (&s[..s.len() - 1], Some(c.to_ascii_uppercase())),
        _ => (s, None),
    };
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "'{s}' is not a size (expected a byte count with an optional K/M/G/T suffix)"
        ));
    }
    let mut value: u128 = 0;
    for b in num.bytes() {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u128::from(b - b'0')))
            .ok_or_else(|| overflow(s))?;
    }
    match suffix {
        None => value.try_into().map(ByteSize).map_err(|_| overflow(s)),
        Some(suf) => {
            let Some(&(_, mult)) = SUFFIXES.iter().find(|&(c, _)| *c == suf) else {
                return Err(format!(
                    "'{s}' is not a size (unknown suffix; expected K, M, G, or T)"
                ));
            };
            value
                .checked_mul(u128::from(mult))
                .and_then(|v| v.try_into().ok())
                .map(ByteSize)
                .ok_or_else(|| overflow(s))
        }
    }
}

fn overflow(s: &str) -> String {
    format!("'{s}' overflows 64 bits")
}

/// True if the size is a non-zero multiple of one mebibyte.
pub fn is_mib_multiple(s: ByteSize) -> bool {
    s.0 > 0 && s.0.is_multiple_of(MIB)
}

/// Human-readable size for plan reports (e.g. `8 GiB`, `512 MiB`,
/// `1.5 GiB`-style values fall back to bytes).
pub fn human(bytes: u64) -> String {
    if bytes >= (1 << 30) && bytes.is_multiple_of(1 << 30) {
        return format!("{} GiB", bytes >> 30);
    }
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        return format!("{} MiB", bytes / MIB);
    }
    format!("{bytes} B")
}

/// Human-readable approximate size for device listings (e.g. `32 GiB`, `1.2 GiB`, `512 MiB`).
pub fn human_approx(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 1.0 {
        if gb.fract() < 0.05 {
            format!("{:.0} GiB", gb)
        } else {
            format!("{:.1} GiB", gb)
        }
    } else {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        format!("{:.0} MiB", mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_bytes() {
        assert_eq!(parse_size("0").unwrap(), ByteSize(0));
        assert_eq!(parse_size("512").unwrap(), ByteSize(512));
        assert_eq!(parse_size("4096").unwrap(), ByteSize(4096));
    }

    #[test]
    fn parses_suffix_sizes_1024_based() {
        assert_eq!(parse_size("512K").unwrap(), ByteSize(512 * 1024));
        assert_eq!(parse_size("1M").unwrap(), ByteSize(1 << 20));
        assert_eq!(parse_size("8G").unwrap(), ByteSize(8 << 30));
        assert_eq!(parse_size("2t").unwrap(), ByteSize(2 << 40));
        assert_eq!(parse_size("1g").unwrap(), ByteSize(1 << 30));
        // no decimal suffixes: "1.5G" is rejected
        assert!(parse_size("1.5G").is_err());
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "", "  ", "X", "1X", "G", "-1", "1 G", "1e3", "0x10", "+5", "512KB",
        ] {
            assert!(parse_size(bad).is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn rejects_overflow() {
        assert!(parse_size("99999999999999999999999999").is_err());
        assert!(parse_size("16Y").is_err());
        assert!(parse_size("18446744073709551616").is_err());
    }

    #[test]
    fn mib_multiple_check() {
        assert!(is_mib_multiple(ByteSize(1 << 20)));
        assert!(!is_mib_multiple(ByteSize(0)));
        assert!(!is_mib_multiple(ByteSize(512 * 1024)));
        assert!(is_mib_multiple(ByteSize(8 << 30)));
    }

    #[test]
    fn human_readable() {
        assert_eq!(human(8 << 30), "8 GiB");
        assert_eq!(human(512 << 20), "512 MiB");
        assert_eq!(human(0), "0 B");
        assert_eq!(human(5000), "5000 B");
    }
}
