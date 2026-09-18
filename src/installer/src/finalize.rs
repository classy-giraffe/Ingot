//! Finalization: sync, copy the install log to the target's /var,
//! unmount, detach the loop device, and replace the original target
//! file with the installed image.
//!
//! Failure-safe teardown (spec 11.6.5): on error the engine detaches
//! everything it attached and leaves the original target file
//! untouched - for file targets the install happened on a working
//! copy, so there is never a partially-written target.

use std::fs;
use std::path::Path;

use crate::log::InstallLog;
use crate::target::DiskTarget;

/// Copies the working log onto the target's /var (the mounted var
/// partition) so the installed system carries the install record.
pub fn copy_log_to_var(log: &InstallLog, var_mount: &Path) -> Result<(), String> {
    let dest = var_mount.join("lib/ingot");
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let src = log.path();
    fs::copy(src, dest.join("install.log"))
        .map_err(|e| format!("cannot copy install log to target /var: {e}"))?;
    Ok(())
}

/// Syncs the file systems (best-effort; `sync` without arguments
/// covers everything the kernel knows about).
pub fn sync_all() {
    let _ = std::process::Command::new("sync").output();
}

/// Best-effort teardown for the failure path: unmount everything,
/// detach the loop, drop the working file. Never fails.
pub fn teardown_failure(target: &mut DiskTarget) {
    let _ = target.detach_all();
    target.cleanup_work_file();
}
