//! The install engine: one validation path, one execution path.
//!
//! `plan` is the read-only half: layout computation, disk size
//! check, artifact resolution with checksum verification, UKI
//! validation. `dry_run` adds a human-readable report; `run` adds
//! the phases (repart, deploy, etcinit, boot, finalize), each logged
//! to the target's /var.
//!
//! Failure policy: for file targets the engine works on a raw copy
//! in the working directory, so an error never leaves the original
//! target partially written; teardown detaches everything and the
//! failure is logged (to the working directory, and to the target's
//! /var when that mount is up).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::boot;
use crate::config::Config;
use crate::deploy;
use crate::etcinit;
use crate::finalize;
use crate::layout::{self, Layout};
use crate::log::InstallLog;
use crate::repart;
use crate::size::human;
use crate::source::{self, Artifact, LiveSource};
use crate::target::{self, DiskTarget};
use crate::ukify;
mod report;
pub use report::render_report;
mod check;
use check::{is_mounted, missing_tools};

/// The validated install plan.
#[derive(Debug)]
pub struct Plan {
    pub cfg: Config,
    pub layout: Layout,
    /// The payload source, per the config's [source] mode.
    pub source: PlanSource,
    pub disk_bytes: u64,
    /// The target's disk format ("qcow2", "raw", or "block device").
    pub disk_format: String,
}

/// The payload source a plan deploys.
#[derive(Debug)]
pub enum PlanSource {
    /// Prebuilt whole-image artifact set (11.4.2).
    Artifacts {
        artifacts: Vec<Artifact>,
        /// Parallel to `artifacts`: the sha256 from the sidecar when
        /// one was present and verified, else `None`.
        checksums: Vec<Option<String>>,
    },
    /// The running release on a mounted ISO media (spec 10.2).
    Live(LiveSource),
}

impl PlanSource {
    /// The artifact of one kind (artifacts mode; complete set).
    fn artifact(&self, kind: &str) -> Option<&Artifact> {
        match self {
            PlanSource::Artifacts { artifacts, .. } => artifacts.iter().find(|a| a.kind == kind),
            PlanSource::Live(_) => None,
        }
    }

    /// The artifact set (artifacts mode).
    fn artifacts(&self) -> &[Artifact] {
        match self {
            PlanSource::Artifacts { artifacts, .. } => artifacts,
            PlanSource::Live(_) => &[],
        }
    }

    /// The parallel checksum list (artifacts mode).
    fn checksums(&self) -> &[Option<String>] {
        match self {
            PlanSource::Artifacts { checksums, .. } => checksums,
            PlanSource::Live(_) => &[],
        }
    }

    /// The UKI the boot phase verifies against (artifacts: the
    /// `efi` artifact; live: the media's installed UKI).
    fn uki(&self) -> &Path {
        match self {
            PlanSource::Artifacts { artifacts, .. } => {
                &artifacts.iter().find(|a| a.kind == "efi").unwrap().path
            }
            PlanSource::Live(live) => &live.uki,
        }
    }
}

/// Disk size without touching the disk: read-only.
fn disk_size(path: &Path) -> Result<(u64, String), String> {
    let meta = fs::metadata(path)
        .map_err(|e| format!("target disk not accessible: {}: {e}", path.display()))?;
    if target::is_block_device(path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let sectors = fs::read_to_string(format!("/sys/block/{name}/size"))
            .map_err(|_| "cannot read /sys/block/<name>/size".to_string())?;
        let sectors: u64 = sectors
            .trim()
            .parse()
            .map_err(|_| "bad /sys/block/<name>/size".to_string())?;
        return Ok((sectors * 512, "block device".to_string()));
    }
    if !meta.is_file() {
        return Err(format!(
            "target disk {} is neither a file nor a block device",
            path.display()
        ));
    }
    let fmt = target::qemu_img_format(path)?;
    let size = target::qemu_img_virtual_size(path)?;
    Ok((size, fmt))
}

fn plan_cfg_encrypted(cfg: &Config) -> bool {
    cfg.enc_var == crate::config::Encryption::Luks2
        || cfg.enc_home == crate::config::Encryption::Luks2
}

/// The read-only validation + plan. Live source mode resolves the
/// release version from the running system's os-release (the running
/// release is the source release).
pub fn plan(cfg: Config) -> Result<Plan, String> {
    plan_with(cfg, None)
}

/// Validates that `path` points to a UKI carrying `version` and the
/// fixed slot A and var PARTUUIDs.
fn validate_uki(path: &Path, version: &crate::version::Version) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("cannot read UKI {}: {e}", path.display()))?;
    ukify::validate_uki(
        &bytes,
        &version.to_string(),
        layout::SLOT_A_UUID,
        layout::VAR_UUID,
    )
    .map_err(|e| format!("UKI validation failed: {e}"))?;
    Ok(())
}

/// The plan with an explicit live-source version (the testable half
/// of [`plan`]: live mode normally derives the version from the
/// running os-release, which tests cannot control).
pub(crate) fn plan_with(
    mut cfg: Config,
    live_version: Option<crate::version::Version>,
) -> Result<Plan, String> {
    if cfg.source_mode == crate::config::SourceMode::Live {
        let version = live_version
            .or_else(|| source::running_version().ok())
            .ok_or_else(|| "live source: cannot determine the running release version".to_string())?;
        cfg.version = version;
    }

    let layout = layout::compute(&cfg);

    // v1 is unencrypted: the config parser accepts the category, the
    // engine rejects what it cannot do (before touching the disk).
    if plan_cfg_encrypted(&cfg) {
        return Err("encryption: luks2 is not supported in v1 (use none)".into());
    }

    let (disk_bytes, disk_format) = disk_size(Path::new(&cfg.target_disk))?;
    layout::check_disk(&layout, disk_bytes)?;

    let base = PathBuf::from(&cfg.source_base);
    let source = match cfg.source_mode {
        crate::config::SourceMode::Artifacts => {
            let artifacts = source::resolve(&base, &cfg.version)?;
            let mut checksums = Vec::with_capacity(artifacts.len());
            for a in &artifacts {
                checksums.push(a.check_sha256()?);
            }
            PlanSource::Artifacts {
                artifacts,
                checksums,
            }
        }
        crate::config::SourceMode::Live => {
            let live = source::resolve_live(&base, &cfg.version)?;
            PlanSource::Live(live)
        }
    };

    validate_uki(source.uki(), &cfg.version)?;

    Ok(Plan {
        cfg,
        layout,
        source,
        disk_bytes,
        disk_format,
    })
}


/// `--dry-run`: validates everything and prints the plan; the disk
/// is never opened for writing.
pub fn dry_run(cfg: Config) -> Result<String, String> {
    let plan = plan(cfg)?;
    Ok(render_report(&plan))
}

/// The full run. Returns a human-readable error on failure; the
/// working directory keeps the log and repart definitions either way.
pub fn run(cfg: Config, work: &Path) -> Result<(), String> {
    // Ingot installs UEFI targets only; the firmware mode of the
    // machine running the install is the target's. Planning and
    // dry-run never require it - only the run that writes the disk.
    if !Path::new("/sys/firmware/efi").exists() {
        return Err(
            "UEFI mode is not active on this system; Ingot installs UEFI targets only".into(),
        );
    }

    // 11.6.1: clearly warn before the destructive operations. In
    // declarative mode the config is the confirmation (11.6.2), but
    // the warning is still mandatory.
    eprintln!(
        "WARNING: this install will destroy all existing content on {}",
        &cfg.target_disk
    );
    let plan = plan(cfg)?;

    // 11.5 phase 1: the required tools must be available before any
    // byte is written (layout::mkfs_set skips the unformatted slot).
    let mut extra = layout::mkfs_set(&plan.layout);
    if plan.disk_format == "qcow2" {
        extra.push("qemu-img".to_string());
    }
    let missing = missing_tools(&std::env::var("PATH").unwrap_or_default(), &extra);
    if !missing.is_empty() {
        return Err(format!(
            "required tools not found in PATH: {} (install the packages that provide them and retry)",
            missing.join(", ")
        ));
    }
    let log = InstallLog::open(work.join("install.log"))
        .map_err(|e| format!("cannot open install log: {e}"))?;
    log.log_result(
        "validate",
        "plan-ok",
        Some(format!(
            "Ingot {} on {} ({}), disk {}",
            plan.cfg.version,
            plan.cfg.target_disk,
            plan.disk_format,
            human(plan.disk_bytes)
        )),
    )?;
    log.log_result(
        "install",
        "target-warning",
        Some(plan.cfg.target_disk.clone()),
    )?;

    let mut target = DiskTarget::prepare(Path::new(&plan.cfg.target_disk), work)?;
    let mut var_mount: Option<PathBuf> = None;
    let r = run_phases(&plan, &mut target, work, &log, &mut var_mount);
    if let Err(e) = r {
        // Failure: log it, keep the record on the target's /var when
        // that mount is still up, then detach everything. The
        // original target file is untouched (the install ran on a
        // working copy).
        let detail = if target.working_copy {
            format!("{e}; original target file left untouched (install ran on a working copy)")
        } else {
            format!("{e}; target is a block device: no automatic rollback")
        };
        let _ = log.log_result("failure", "install-failed", Some(detail));
        if let Some(vm) = &var_mount {
            if is_mounted(vm) {
                let _ = finalize::copy_log_to_var(&log, vm);
            }
        }
        finalize::sync_all();
        finalize::teardown_failure(&mut target);
        return Err(e);
    }
    Ok(())
}


fn run_phases(
    plan: &Plan,
    target: &mut DiskTarget,
    work: &Path,
    log: &InstallLog,
    var_mount: &mut Option<PathBuf>,
) -> Result<(), String> {
    let v = &plan.cfg.version;

    // 1. repart the working disk, then loop-attach with partscan so
    //    the kernel sees the fresh GPT immediately
    repart::run(&plan.layout, &work.join("defs"), target, log)?;
    target.attach_loop()?;
    // Block-device targets need an explicit rescan (loop devices pick
    // up the GPT at attach via -P; partprobe is a no-op for them).
    // Best-effort: the outcome is logged either way.
    match std::process::Command::new("partprobe")
        .arg(target.active_device())
        .output()
    {
        Ok(out) if out.status.success() => log.log_result("repart", "partprobe", None)?,
        Ok(out) => log.log_result(
            "repart",
            "partprobe-warn",
            Some(format!(
                "partprobe exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        )?,
        Err(e) => log.log_result(
            "repart",
            "partprobe-skip",
            Some(format!("partprobe not usable: {e}")),
        )?,
    }

    // 2. partition devices (slot B is looked up for the record only)
    let parts = partition_devices(plan, target, log, var_mount)?;

    // 3. payload deployment: whole-image artifacts, or the live
    //    source (the running release's erofs + the media's ESP tree).
    //    Live mode mounts the ESP early: composing the tree needs a
    //    mount point (the phase-4 mount step then reuses it).
    let mut esp_mount: Option<PathBuf> = None;
    match &plan.source {
        PlanSource::Artifacts { .. } => {
            deploy::deploy(&plan.source.artifacts()[..], &parts, log)?;
        }
        PlanSource::Live(live) => {
            let esp_dev = parts
                .get("esp.raw")
                .expect("layout always contains the ESP")
                .clone();
            esp_mount = Some(target.mount(&esp_dev, "vfat", work, "")?);
            let esp_m = esp_mount.as_ref().unwrap();
            deploy::deploy_live(live, &parts, esp_m, log)?;
        }
    }

    // 4. mounts for content initialization
    let (var_m, home_m, esp_m, slot_m) =
        mount_for_init(plan, target, &parts, work, var_mount, &mut esp_mount)?;

    // 5. /var/lib/etc from factory defaults + config
    etcinit::run(&plan.cfg, &var_m, &home_m, &slot_m, log)?;

    // 6. ESP / UKI verification
    boot::verify(&esp_m, plan.source.uki(), v, log)?;

    // 7. finalize
    log.log_result(
        "finalize",
        "install-complete",
        Some(format!(
            "Ingot {v} installed; target replaced ({} virtual)",
            human(plan.disk_bytes)
        )),
    )?;
    finalize::copy_log_to_var(log, &var_m)?;
    finalize::sync_all();
    target
        .detach_all()
        .map_err(|e| format!("teardown failed: {e}"))?;
    target
        .commit()
        .map_err(|e| format!("cannot install over the target: {e}"))?;
    target.cleanup_work_file();
    Ok(())
}

/// Phase 2: look up the partition devices by their fixed PARTUUIDs
/// (slot B is looked up for the record only) and log each one ready.
fn partition_devices(
    plan: &Plan,
    target: &mut DiskTarget,
    log: &InstallLog,
    var_mount: &mut Option<PathBuf>,
) -> Result<BTreeMap<&'static str, PathBuf>, String> {
    let mut parts: BTreeMap<&'static str, PathBuf> = BTreeMap::new();
    for p in &plan.layout.partitions {
        let dev = target.partition_by_uuid(p.part_uuid, 15)?;
        log.log_result(
            "repart",
            "partition-ready",
            Some(format!(
                "{} -> {} ({} {})",
                p.label,
                dev.display(),
                p.fs,
                p.size
            )),
        )?;
        match p.role {
            layout::Role::Esp => {
                parts.insert("esp.raw", dev);
            }
            layout::Role::SlotA => {
                parts.insert("slot.raw", dev);
            }
            layout::Role::Var => {
                parts.insert("var.raw", dev.clone());
                *var_mount = Some(dev);
            }
            layout::Role::Home => {
                parts.insert("home.raw", dev);
            }
            layout::Role::SlotB => {}
        }
    }
    Ok(parts)
}

/// Phase 4: mount the partitions for content initialization: /var and
/// /home read-write, the ESP read-write, the active slot read-only.
/// The ESP is reused when `esp_mount` already holds it (live mode
/// mounts it during deployment).
fn mount_for_init(
    plan: &Plan,
    target: &mut DiskTarget,
    parts: &BTreeMap<&'static str, PathBuf>,
    work: &Path,
    var_mount: &mut Option<PathBuf>,
    esp_mount: &mut Option<PathBuf>,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    // Look the filesystems up by role: the partition order is a
    // layout.rs invariant, not something the engine re-encodes.
    let fs_of = |role: layout::Role| -> &str {
        plan.layout
            .partitions
            .iter()
            .find(|p| p.role == role)
            .expect("the layout always contains every role")
            .fs
            .as_str()
    };
    let var_fs = fs_of(layout::Role::Var);
    let home_fs = fs_of(layout::Role::Home);
    let var_dev = parts.get("var.raw").unwrap().clone();
    let home_dev = parts.get("home.raw").unwrap().clone();
    let esp_dev = parts.get("esp.raw").unwrap().clone();
    let slot_dev = parts.get("slot.raw").unwrap().clone();
    let var_m = target.mount(&var_dev, var_fs, work, "")?;
    *var_mount = Some(var_m.clone());
    let home_m = target.mount(&home_dev, home_fs, work, "")?;
    let esp_m = match esp_mount.take() {
        Some(m) => m,
        None => target.mount(&esp_dev, "vfat", work, "")?,
    };
    let slot_m = target.mount(&slot_dev, "erofs", work, "ro")?;
    Ok((var_m, home_m, esp_m, slot_m))
}

#[cfg(test)]
mod tests;
