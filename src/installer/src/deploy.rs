//! Whole-image deployment: `dd` of the slot, ESP, /var and /home
//! artifacts onto their partitions.
//!
//! The artifacts are complete filesystem images smaller than or equal
//! to their partitions; the remainder of a slot partition stays
//! unformatted. Slot B gets no image at install time: it is the free
//! `_empty` slot an update fills later. The UKI artifact is not
//! dd'ed (the ESP image already carries it); `boot.rs` re-checks it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::Digest;

use crate::log::InstallLog;
use crate::source::{Artifact, LiveSource};

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
/// Live-source deployment (spec 10.2): the running release's erofs is
/// `dd`'d onto slot A (hash-verified, like the artifact slot image),
/// and the media's `esp/` tree (systemd-boot fallback, the installed
/// UKI, the loader config) is copied onto the already-formatted ESP.
/// The state partitions (var, home) are left as repart formatted them
/// (btrfs/ext4, per the config): the factory state is empty, and
/// etcinit initializes them from the slot's factory defaults.
pub fn deploy_live(
    src: &LiveSource,
    parts: &std::collections::BTreeMap<&'static str, PathBuf>,
    esp_mount: &Path,
    log: &InstallLog,
) -> Result<(), String> {
    // Slot A: the running release's erofs payload.
    let dev = parts
        .get("slot.raw")
        .ok_or_else(|| "no partition device for the slot (live deploy)".to_string())?;
    let meta = fs::metadata(&src.erofs)
        .map_err(|e| format!("cannot stat live payload {}: {e}", src.erofs.display()))?;
    let art = Artifact {
        kind: "slot.raw",
        path: src.erofs.clone(),
        size: meta.len(),
    };
    place("slot A (live payload)", &art, dev, log)?;

    // ESP: the media's installed-ESP tree onto the formatted ESP.
    copy_tree(&src.esp_tree, esp_mount)
        .map_err(|e| format!("cannot compose the ESP from {}: {e}", src.esp_tree.display()))?;
    log.log_result(
        "deploy",
        "esp-composed",
        Some(format!(
            "{} -> {} (systemd-boot + UKI + loader)",
            src.esp_tree.display(),
            esp_mount.display()
        )),
    )?;
    Ok(())
}

/// Recursively copies `src` into `dst` (files and directories),
/// preserving permissions and modes. Symlinks are copied as their
/// target contents (the ESP tree carries no symlinks; the live payload
/// is erofs, not copied tree-by-tree).
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(src)
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst)
            .map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
        fs::set_permissions(dst, fs::Permissions::from_mode(meta.permissions().mode()))
            .map_err(|e| format!("cannot chmod {}: {e}", dst.display()))?;
        for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_tree(entry.path().as_path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::create_dir_all(dst.parent().unwrap_or(dst))
            .map_err(|e| format!("cannot create parent of {}: {e}", dst.display()))?;
        fs::copy(src, dst)
            .map_err(|e| format!("cannot copy {} -> {}: {e}", src.display(), dst.display()))?;
        fs::set_permissions(dst, fs::Permissions::from_mode(meta.permissions().mode()))
            .map_err(|e| format!("cannot chmod {}: {e}", dst.display()))
    }
}
