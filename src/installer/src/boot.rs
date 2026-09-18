//! Boot verification: after the ESP image is deployed, re-check the
//! UKI on the mounted ESP and (best-effort) ask `bootctl` about the
//! loader state.
//!
//! The ESP artifact was built at release time with systemd-boot and
//! the UKI already in place (spec 6: the installer places pre-built
//! artifacts and does not build UKIs at install time), so this phase
//! is verification, not construction.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::log::InstallLog;
use crate::source::Artifact;
use crate::version::Version;

/// Verifies the deployed ESP: the UKI is present at the expected
/// path with the exact artifact bytes, and `bootctl status` (if the
/// host has it) sees the ESP.
pub fn verify(
    esp_mount: &Path,
    uki: &Artifact,
    version: &Version,
    log: &InstallLog,
) -> Result<(), String> {
    let uki_path = esp_mount.join(format!("EFI/Linux/ingot_{version}.efi"));
    if !uki_path.is_file() {
        return Err(format!(
            "UKI missing from deployed ESP at {}",
            uki_path.display()
        ));
    }
    let on_disk = fs::read(&uki_path).map_err(|e| format!("cannot read deployed UKI: {e}"))?;
    let expected = fs::read(&uki.path).map_err(|e| format!("cannot read UKI artifact: {e}"))?;
    if on_disk != expected {
        return Err(format!(
            "deployed UKI {} does not match the release artifact {} (bytes differ)",
            uki_path.display(),
            uki.path.display()
        ));
    }
    log.log_result(
        "boot",
        "uki-verified",
        Some(format!("{} ({} bytes)", uki_path.display(), on_disk.len())),
    )?;

    // The ESP should carry the systemd-boot loader.
    let loader = esp_mount.join("EFI/BOOT/BOOTX64.EFI");
    if !loader.is_file() {
        return Err(format!(
            "systemd-boot loader missing from deployed ESP at {}",
            loader.display()
        ));
    }

    // Best-effort bootctl: the host running the installer may not
    // have it; its failure never fails the install.
    let e = esp_mount.to_string_lossy().to_string();
    match Command::new("bootctl")
        .args(["--esp-path", &e, "status"])
        .output()
    {
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
            .trim()
            .to_string();
            log.log_result("boot", "bootctl-status", Some(text))?;
        }
        Err(e) => log.log_result(
            "boot",
            "bootctl-skip",
            Some(format!("bootctl not usable on the installer host: {e}")),
        )?,
    }
    Ok(())
}
