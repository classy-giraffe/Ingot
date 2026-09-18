//! Whole-image deployment: `dd` of the slot, ESP, /var and /home
//! artifacts onto their partitions.
//!
//! The artifacts are complete filesystem images smaller than or equal
//! to their partitions; the remainder of a slot partition stays
//! unformatted. Slot B gets no image at install time: it is the free
//! `_empty` slot an update fills later. The UKI artifact is not
//! dd'ed (the ESP image already carries it); `boot.rs` re-checks it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::Digest;

use crate::log::InstallLog;
use crate::source::Artifact;

/// Deploys one artifact onto a partition, then verifies the written
/// bytes by hashing the first `art.size` bytes of the device and
/// comparing with the artifact's hash.
fn place(what: &str, art: &Artifact, dev: &Path, log: &InstallLog) -> Result<(), String> {
    let d = dev.to_string_lossy().to_string();
    let a = art.path.to_string_lossy().to_string();
    let out = Command::new("dd")
        .args([
            &format!("if={a}"),
            &format!("of={d}"),
            "bs=4M",
            "conv=fsync",
            "status=none",
        ])
        .output()
        .map_err(|e| format!("cannot run dd: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "dd of {what} onto {} failed: {}",
            dev.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    log.log_result(
        "deploy",
        "written",
        Some(format!("{what} -> {} ({} bytes)", dev.display(), art.size)),
    )?;

    // Verification: hash the first art.size device bytes in-process.
    let mut h = sha2::Sha256::new();
    let mut f = fs::File::open(dev)
        .map_err(|e| format!("cannot re-open {} for verification: {e}", dev.display()))?;
    let mut remaining = art.size;
    let mut buf = [0u8; 65536];
    while remaining > 0 {
        let n = std::io::Read::read(&mut f, &mut buf)
            .map_err(|e| format!("verification read of {} failed: {e}", dev.display()))?;
        if n == 0 {
            return Err(format!(
                "verification failed: {} is shorter than the {} artifact ({} < {} bytes)",
                dev.display(),
                what,
                art.size - remaining,
                art.size
            ));
        }
        let take = (n as u64).min(remaining) as usize;
        h.update(&buf[..take]);
        remaining -= take as u64;
    }
    let dev_sum = format!("{:x}", h.finalize());
    let art_sum = crate::source::sha256_file(&art.path)
        .map_err(|e| format!("cannot hash artifact for verification: {e}"))?;
    if dev_sum != art_sum {
        return Err(format!(
            "verification failed: {} on {} does not match {}",
            what,
            dev.display(),
            art.path.display()
        ));
    }
    log.log_result("deploy", "verified", Some(what.to_string()))?;
    Ok(())
}

/// Deploys the artifact set. `parts` maps artifact kind to partition
/// device; slot B is absent (it stays empty).
pub fn deploy(
    artifacts: &[Artifact],
    parts: &std::collections::BTreeMap<&'static str, PathBuf>,
    log: &InstallLog,
) -> Result<(), String> {
    for art in artifacts {
        if art.kind == "efi" {
            continue;
        }
        let dev = parts
            .get(art.kind)
            .ok_or_else(|| format!("no partition device for artifact {}", art.kind))?;
        place(art.kind, art, dev, log)?;
    }
    Ok(())
}
