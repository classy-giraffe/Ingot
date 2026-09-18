//! Artifact resolution and integrity checks.
//!
//! A release version ships as a set of whole-image artifacts under a
//! base directory (the `source` category, spec 11.4.2):
//!
//! | kind     | file                    | deployed to      |
//! |----------|-------------------------|------------------|
//! | slot     | `ingot_<v>.slot.raw`    | slot A partition |
//! | efi      | `ingot_<v>.efi`         | the UKI (checked; the ESP image already carries it) |
//! | esp      | `ingot_<v>.esp.raw`     | ESP partition    |
//! | var      | `ingot_<v>.var.raw`     | /var partition   |
//! | home     | `ingot_<v>.home.raw`    | /home partition  |
//!
//! If a `.sha256` sidecar (sha256sum format: `<hex>  <name>`) sits
//! next to an artifact, the artifact's checksum is verified.

use crate::version::Version;
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};

/// The artifact kinds, in deploy order.
pub const KINDS: [&str; 5] = ["slot.raw", "efi", "esp.raw", "var.raw", "home.raw"];

/// One resolved artifact.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub kind: &'static str,
    pub path: PathBuf,
    pub size: u64,
}

impl Artifact {
    /// Verifies the `.sha256` sidecar if one exists next to the
    /// artifact. Returns the checksum that was checked (or `None`
    /// when no sidecar is present).
    pub fn check_sha256(&self) -> Result<Option<String>, String> {
        let sidecar = self.path.with_file_name(format!(
            "{}.sha256",
            self.path.file_name().unwrap().to_string_lossy()
        ));
        let Some(content) = fs::read_to_string(&sidecar).ok() else {
            return Ok(None);
        };
        // sha256sum format: `<hex>  <name>` or `<hex> *<name>`
        let mut parts = content
            .lines()
            .next()
            .unwrap_or("")
            .splitn(2, char::is_whitespace);
        let hex = parts.next().unwrap_or("");
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "malformed checksum sidecar {}: expected sha256sum format",
                sidecar.display()
            ));
        }
        let actual = crate::source::sha256_file(&self.path)
            .map_err(|e| format!("cannot hash {}: {e}", self.path.display()))?;
        if actual != hex {
            return Err(format!(
                "checksum mismatch for {}: sidecar says {hex}, file is {actual}",
                self.path.display()
            ));
        }
        Ok(Some(hex.to_string()))
    }
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut h = sha2::Sha256::new();
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 65536];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Resolves the full artifact set for a version under `base`.
/// Errors name the missing file.
pub fn resolve(base: &Path, version: &Version) -> Result<Vec<Artifact>, String> {
    let mut out = Vec::with_capacity(KINDS.len());
    for kind in KINDS {
        let path = base.join(format!("ingot_{version}.{kind}"));
        let meta = match fs::metadata(&path) {
            Ok(m) if m.is_file() => m,
            _ => {
                return Err(format!(
                    "artifact not found: {} (expected in {})",
                    path.display(),
                    base.display()
                ));
            }
        };
        out.push(Artifact {
            kind,
            path,
            size: meta.len(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ingot-src-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resolve_finds_all_and_names_the_missing_one() {
        let d = tmp("resolve");
        let v = Version {
            major: 0,
            minor: 1,
            patch: 0,
        };
        for k in KINDS {
            fs::write(d.join(format!("ingot_0.1.0.{k}")), b"x").unwrap();
        }
        let arts = resolve(&d, &v).unwrap();
        assert_eq!(arts.len(), 5);
        assert_eq!(
            arts.iter().map(|a| a.kind).collect::<Vec<_>>(),
            KINDS.to_vec()
        );
        fs::remove_file(d.join("ingot_0.1.0.esp.raw")).unwrap();
        let e = resolve(&d, &v).unwrap_err();
        assert!(e.contains("esp.raw"), "{e}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sha256_sidecar_verified() {
        let d = tmp("sha");
        let f = d.join("ingot_0.1.0.slot.raw");
        fs::write(&f, b"payload").unwrap();
        let sum = sha256_file(&f).unwrap();
        fs::write(d.join("ingot_0.1.0.slot.raw.sha256"), format!("{sum}  x\n")).unwrap();
        let a = Artifact {
            kind: "slot.raw",
            path: f.clone(),
            size: 7,
        };
        assert_eq!(a.check_sha256().unwrap().as_deref(), Some(sum.as_str()));
        // corrupt the file -> mismatch
        fs::write(&f, b"payload!").unwrap();
        let e = a.check_sha256().unwrap_err();
        assert!(e.contains("mismatch"), "{e}");
        // malformed sidecar -> error
        fs::write(d.join("ingot_0.1.0.slot.raw.sha256"), "nothex  x\n").unwrap();
        assert!(a.check_sha256().unwrap_err().contains("malformed"));
        // no sidecar -> Ok(None)
        fs::remove_file(d.join("ingot_0.1.0.slot.raw.sha256")).unwrap();
        assert_eq!(a.check_sha256().unwrap(), None);
        let _ = fs::remove_dir_all(&d);
    }
}
