//! SHA256SUMS parsing: the strict GNU sha256sum(1) format that
//! systemd-sysupdate requires.

use anyhow::bail;
use std::collections::HashSet;

/// One `<sha256> <name>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SumsEntry {
    pub name: String,
    pub sha256: String,
}

/// Whether `sha` is a 64-character lowercase-hex sha256 (the strict
/// form both `SHA256SUMS` and the manifest asset array require).
pub fn is_sha256_hex(sha: &str) -> bool {
    sha.len() == 64
        && sha
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Parses `SHA256SUMS` content.
///
/// Format (per sysupdate's parser): 64 lowercase-hex sha256, a space,
/// a text (` `) or binary (`*`) marker, the file name, a newline; a
/// final newline is required. Asset names are flat (no subpaths: on
/// GitHub Releases an asset name is a single URL segment). A
/// `BEST-BEFORE-YYYY-MM-DD` freshness entry (a sysupdate feature, not
/// an asset) is tolerated and excluded from the asset list.
pub fn parse(data: &[u8]) -> anyhow::Result<Vec<SumsEntry>> {
    let text = std::str::from_utf8(data)
        .map_err(|_| anyhow::anyhow!("SHA256SUMS: not valid UTF-8"))?;
    if !text.ends_with('\n') {
        bail!("SHA256SUMS: missing final newline");
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (n, line) in text.lines().enumerate() {
        let line_no = n + 1;
        if line.is_empty() {
            bail!("SHA256SUMS: empty line {line_no}");
        }
        if line.len() < 66 {
            bail!("SHA256SUMS: malformed entry on line {line_no}");
        }
        let (sha256, rest) = line.split_at(64);
        if !is_sha256_hex(sha256) {
            bail!("SHA256SUMS: malformed hash on line {line_no}");
        }
        let (marker, name) = rest.split_at(2);
        match marker {
            "  " | " *" => {}
            _ => bail!("SHA256SUMS: bad marker on line {line_no}"),
        }
        if name.is_empty() {
            bail!("SHA256SUMS: missing file name on line {line_no}");
        }
        if name.contains('\0')
            || name.bytes().any(|b| b < 0x20 || b == 0x7f)
            || name.contains('/')
            || name.contains('\\')
        {
            bail!("SHA256SUMS: invalid file name on line {line_no}");
        }
        // Freshness marker (sysupdate feature): not an asset.
        if name.starts_with("BEST-BEFORE-") {
            continue;
        }
        if !seen.insert(name) {
            bail!("SHA256SUMS: duplicate entry for {name}");
        }
        out.push(SumsEntry {
            name: name.to_string(),
            sha256: sha256.to_string(),
        });
    }
    Ok(out)
}
