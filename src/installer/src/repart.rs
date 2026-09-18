//! Partitioning: writes the layout's systemd-repart definitions and
//! runs `systemd-repart` on the target.
//!
//! The definitions are rendered by `layout::repart_defs` (fixed
//! UUIDs, exact sizes, slot B left unformatted). `--empty=create`
//! creates a fresh GPT on the empty working disk.

use std::fs;
use std::path::Path;

use crate::layout;
use crate::log::InstallLog;
use crate::target::DiskTarget;

/// Writes the definitions to `defs_dir` and runs systemd-repart.
pub fn run(
    layout: &layout::Layout,
    defs_dir: &Path,
    target: &DiskTarget,
    log: &InstallLog,
) -> Result<(), String> {
    fs::create_dir_all(defs_dir).map_err(|e| {
        format!(
            "cannot create definitions directory {}: {e}",
            defs_dir.display()
        )
    })?;
    for (name, conf) in layout::repart_defs(layout) {
        fs::write(defs_dir.join(&name), conf)
            .map_err(|e| format!("cannot write repart definition {name}: {e}"))?;
    }
    log.log_result(
        "repart",
        "defs-written",
        Some(format!(
            "{} definitions in {}",
            layout.partitions.len(),
            defs_dir.display()
        )),
    )?;

    let disk = target.active_device().to_string_lossy().to_string();
    let defs = defs_dir.to_string_lossy().to_string();
    // The device is attached and zero-filled (fresh install):
    // require writes the fresh GPT on the empty disk and fails if a
    // partition table is already there (never overwrite silently).
    let out = std::process::Command::new("systemd-repart")
        .args([
            "--definitions",
            &defs,
            "--empty=require",
            "--dry-run=no",
            "--",
            &disk,
        ])
        .output()
        .map_err(|e| format!("cannot run systemd-repart: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "systemd-repart failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    log.log_result("repart", "repart-done", None)?;
    Ok(())
}
