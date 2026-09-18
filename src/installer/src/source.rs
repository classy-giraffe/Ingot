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

use crate::version::{self, Version};
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
/// The live payload source (spec 10.2, live ISO): the running
/// release on a mounted ISO media. The media layout (the ISO build's
/// output):
///
/// | what         | path                             |
/// |--------------|----------------------------------|
/// | slot erofs   | `LiveOS/rootfs.erofs`            |
/// | installed UKI| `esp/EFI/Linux/ingot_<v>.efi`    |
/// | ESP tree     | `esp/`                           |
///
/// The slot erofs IS the running /usr (the live payload); deploying
/// it to slot A makes the installed system run the same release. The
/// `esp/` tree is the installed machine's ESP (systemd-boot fallback,
/// the installed UKI, the loader config) - the live boot's own ESP
/// tree (at the media root) additionally carries the live UKI and a
/// live-default loader.conf, and is not what the install deploys.
#[derive(Debug, Clone)]
pub struct LiveSource {
    /// The running release's erofs payload (deployed to slot A).
    pub erofs: PathBuf,
    /// The installed UKI (validated for the fixed PARTUUIDs).
    pub uki: PathBuf,
    /// The installed ESP tree (copied onto the formatted ESP).
    pub esp_tree: PathBuf,
    /// sha256 of the erofs payload (deployment verification).
    pub erofs_sha: String,
    /// sha256 of the installed UKI.
    pub uki_sha: String,
}

/// The live payload path on the ISO media (spec 10.3 layout).
pub const LIVE_EROFS: &str = "LiveOS/rootfs.erofs";

/// Resolves the live payload source under the mounted ISO media
/// `base` for `version`. Errors name the missing file.
pub fn resolve_live(base: &Path, version: &Version) -> Result<LiveSource, String> {
    let erofs = base.join(LIVE_EROFS);
    let esp_tree = base.join("esp");
    let uki = esp_tree
        .join("EFI/Linux")
        .join(format!("ingot_{version}.efi"));
    for p in [&erofs, &uki] {
        if !p.is_file() {
            return Err(format!(
                "live source: {} not found under {}",
                p.display(),
                base.display()
            ));
        }
    }
    if !esp_tree.is_dir() {
        return Err(format!(
            "live source: ESP tree {} not found under {}",
            esp_tree.display(),
            base.display()
        ));
    }
    let erofs_sha = sha256_file(&erofs)
        .map_err(|e| format!("live source: cannot hash {}: {e}", erofs.display()))?;
    let uki_sha = sha256_file(&uki)
        .map_err(|e| format!("live source: cannot hash {}: {e}", uki.display()))?;
    Ok(LiveSource {
        erofs,
        uki,
        esp_tree,
        erofs_sha,
        uki_sha,
    })
}

/// The running release version from the os-release of the system the
/// installer runs on (live source mode: the running release is the
/// source release). Reads /usr/lib/os-release (falling back to
/// /etc/os-release) and parses its `VERSION_ID` line.
pub fn running_version() -> Result<Version, String> {
    let text = fs::read_to_string("/usr/lib/os-release")
        .or_else(|_| fs::read_to_string("/etc/os-release"))
        .map_err(|e| format!("live source: cannot read os-release: {e}"))?;
    parse_os_release(&text)
        .ok_or_else(|| "live source: no VERSION_ID in os-release".to_string())
}

/// Parses `VERSION_ID` out of an os-release document (the pure half
/// of [`running_version`], unit-testable).
pub fn parse_os_release(text: &str) -> Option<Version> {
    text.lines()
        .find_map(|l| l.strip_prefix("VERSION_ID="))
        .map(|v| v.trim_matches('"').to_string())
        .and_then(|v| version::parse(&v).ok())
}


/// The Ingot versions with a slot artifact present under `base`,
/// newest first. The wizard's default version source; entries that
/// are not regular files or do not name a version are skipped.
pub fn available_versions(base: &Path) -> Vec<Version> {
    let Ok(rd) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let file_name = e.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(v) = name
            .strip_prefix("ingot_")
            .and_then(|rest| rest.strip_suffix(".slot.raw"))
        else {
            continue;
        };
        let Ok(m) = e.metadata() else {
            continue;
        };
        if m.is_file() {
            if let Ok(v) = crate::version::parse(v) {
                out.push(v);
            }
        }
    }
    out.sort_by(|a, b| b.cmp(a));
    out
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
    fn available_versions_lists_slot_releases_newest_first() {
        let d = tmp("versions");
        for v in ["0.1.0", "0.2.0", "1.9.0", "1.10.0"] {
            fs::write(d.join(format!("ingot_{v}.slot.raw")), b"x").unwrap();
        }
        // Not a slot artifact, not a file, or not a version: ignored.
        fs::write(d.join("ingot_0.3.0.efi"), b"x").unwrap();
        fs::write(d.join("ingot_9.9.9.usr-x86-64.raw"), b"x").unwrap();
        fs::write(d.join("ingot_notaversion.slot.raw"), b"x").unwrap();
        fs::create_dir(d.join("ingot_2.0.0.slot.raw")).unwrap();
        let vs = available_versions(&d);
        assert_eq!(
            vs,
            vec![
                Version { major: 1, minor: 10, patch: 0 },
                Version { major: 1, minor: 9, patch: 0 },
                Version { major: 0, minor: 2, patch: 0 },
                Version { major: 0, minor: 1, patch: 0 },
            ]
        );
        assert!(available_versions(&d.join("no-such-dir")).is_empty());
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

    /// A fixture media: the live payload erofs and the installed ESP
    /// tree (with the versioned UKI).
    fn live_media(d: &Path, version: &str) {
        let erofs = d.join(LIVE_EROFS);
        fs::create_dir_all(erofs.parent().unwrap()).unwrap();
        fs::write(&erofs, b"erofs").unwrap();
        let uki = d
            .join("esp")
            .join("EFI/Linux")
            .join(format!("ingot_{version}.efi"));
        fs::create_dir_all(uki.parent().unwrap()).unwrap();
        fs::write(&uki, b"uki").unwrap();
        fs::create_dir_all(d.join("esp/loader")).unwrap();
        fs::write(d.join("esp/loader/loader.conf"), "timeout 3\n").unwrap();
    }

    #[test]
    fn resolve_live_finds_media_files() {
        let d = tmp("live");
        let v = Version {
            major: 0,
            minor: 1,
            patch: 0,
        };
        live_media(&d, "0.1.0");
        let s = resolve_live(&d, &v).unwrap();
        assert_eq!(s.erofs, d.join(LIVE_EROFS));
        assert_eq!(
            s.uki,
            d.join("esp/EFI/Linux/ingot_0.1.0.efi")
        );
        assert_eq!(s.esp_tree, d.join("esp"));
        assert_eq!(s.erofs_sha, sha256_file(&d.join(LIVE_EROFS)).unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_live_names_the_missing_file() {
        let d = tmp("live-missing");
        let v = Version {
            major: 0,
            minor: 1,
            patch: 0,
        };
        live_media(&d, "0.1.0");
        fs::remove_file(d.join(LIVE_EROFS)).unwrap();
        let e = resolve_live(&d, &v).unwrap_err();
        assert!(e.contains(LIVE_EROFS), "{e}");
        // restore the payload; drop the UKI -> the UKI is named
        fs::write(d.join(LIVE_EROFS), b"erofs").unwrap();
        fs::remove_file(d.join("esp/EFI/Linux/ingot_0.1.0.efi")).unwrap();
        let e = resolve_live(&d, &v).unwrap_err();
        assert!(e.contains("ingot_0.1.0.efi"), "{e}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn parse_os_release_reads_version_id() {
        assert_eq!(
            parse_os_release("NAME=Ingot\nVERSION_ID=\"0.1.0\"\nID=ingot\n"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
        assert_eq!(
            parse_os_release("ID=ingot\n"),
            None
        );
        // unquoted VERSION_ID also parses
        assert_eq!(
            parse_os_release("VERSION_ID=1.2.3\n"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

}
