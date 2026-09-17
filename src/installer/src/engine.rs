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
use crate::source::{self, Artifact};
use crate::target::{self, DiskTarget};
use crate::ukify;

/// The validated install plan.
#[derive(Debug)]
pub struct Plan {
    pub cfg: Config,
    pub layout: Layout,
    pub artifacts: Vec<Artifact>,
    /// Parallel to `artifacts`: the sha256 from the sidecar when one
    /// was present and verified, else `None`.
    pub checksums: Vec<Option<String>>,
    pub disk_bytes: u64,
    /// The target's disk format ("qcow2", "raw", or "block device").
    pub disk_format: String,
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
    let fmt = crate::target::qemu_img_format(path)?;
    let v: serde_json::Value = crate::target::qemu_img_info(path)?
        .parse()
        .map_err(|_| "qemu-img info: bad JSON".to_string())?;
    let size = v
        .get("virtual-size")
        .and_then(|x| x.as_u64())
        .ok_or("qemu-img info: no virtual-size")?;
    Ok((size, fmt))
}

fn plan_cfg_encrypted(cfg: &Config) -> bool {
    cfg.enc_var == crate::config::Encryption::Luks2
        || cfg.enc_home == crate::config::Encryption::Luks2
}

/// The read-only validation + plan.
pub fn plan(cfg: Config) -> Result<Plan, String> {
    let layout = layout::compute(&cfg);

    // v1 is unencrypted: the config parser accepts the category, the
    // engine rejects what it cannot do (before touching the disk).
    if plan_cfg_encrypted(&cfg) {
        return Err("encryption: luks2 is not supported in v1 (use none)".into());
    }

    let (disk_bytes, disk_format) = disk_size(Path::new(&cfg.target_disk))?;
    layout::check_disk(&layout, disk_bytes)?;

    let base = PathBuf::from(&cfg.source_base);
    let artifacts = source::resolve(&base, &cfg.version)?;
    let mut checksums = Vec::with_capacity(artifacts.len());
    for a in &artifacts {
        checksums.push(a.check_sha256()?);
    }
    // UKI: the fixed PARTUUIDs and the version must be baked in.
    let uki = artifacts
        .iter()
        .find(|a| a.kind == "efi")
        .expect("artifact set always contains the efi");
    let bytes = fs::read(&uki.path)
        .map_err(|e| format!("cannot read UKI artifact {}: {e}", uki.path.display()))?;
    let version_str = cfg.version.to_string();
    ukify::validate_uki(
        &bytes,
        &version_str,
        layout::SLOT_A_UUID,
        layout::VAR_UUID,
    )
    .map_err(|e| format!("UKI validation failed: {e}"))?;

    Ok(Plan {
        cfg,
        layout,
        checksums,
        artifacts,
        disk_bytes,
        disk_format,
    })
}

/// The dry-run report (stdout), covering partitions, sizes, and
/// deployments.
pub fn render_report(plan: &Plan) -> String {
    let mut s = String::new();
    let v = &plan.layout.version;
    s.push_str(&format!("ingot-installer dry run: plan for Ingot {v}\n\n"));
    s.push_str(&format!(
        "Target: {} (format: {}, {} virtual)\n\n",
        plan.cfg.target_disk,
        plan.disk_format,
        human(plan.disk_bytes)
    ));
    s.push_str("Partitions (fixed UUIDs, exact sizes):\n");
    for p in &plan.layout.partitions {
        s.push_str(&format!(
            "  #{} {:<13} {}  {:<13} {:>10} @ {}\n",
            p.index,
            p.label,
            p.part_uuid,
            p.fs,
            human(p.size),
            human(p.offset)
        ));
    }
    s.push_str(&format!(
        "  total {} ({} required)\n\n",
        human(plan.layout.used_bytes),
        human(plan.layout.required_disk_bytes)
    ));
    s.push_str("Deployment:\n");
    for (kind, part, what) in [
        ("slot.raw", "slot A", "active version image"),
        ("esp.raw", "esp", "systemd-boot + UKI"),
        ("var.raw", "var", "factory state"),
        ("home.raw", "home", "factory state"),
    ] {
        let art = plan
            .artifacts
            .iter()
            .find(|a| a.kind == kind)
            .expect("artifact set is complete");
        s.push_str(&format!(
            "  ingot_{v}.{kind} ({} bytes) -> {part} ({what})\n",
            human(art.size)
        ));
    }
    s.push_str("  slot B stays _empty (unformatted; an update fills it)\n\n");
    s.push_str("Artifacts (sha256 from .sha256 sidecars when present):\n");
    for (a, sum) in plan.artifacts.iter().zip(&plan.checksums) {
        let sum = sum.clone().unwrap_or_else(|| "- (no sidecar)".to_string());
        s.push_str(&format!("  ingot_{v}.{}  {sum}\n", a.kind));
    }
    s.push_str(&format!(
        "\nUKI ingot_{v}.efi: valid (VERSION_ID {v}, fixed PARTUUIDs in command line)\n"
    ));
    s.push_str(&format!(
        "System: hostname={} timezone={} locale={} keymap={}\n",
        plan.cfg.hostname, plan.cfg.timezone, plan.cfg.locale, plan.cfg.keymap
    ));
    let users: Vec<String> = plan
        .cfg
        .users
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let shell = u
                .shell
                .clone()
                .unwrap_or_else(|| crate::config::DEFAULT_SHELL.to_string());
            format!("{} (uid {}, {})", u.name, 1000 + i as u32, shell)
        })
        .collect();
    s.push_str(&format!("Users: {}\n", users.join(", ")));
    s.push_str(&format!(
        "SSH keys: {} authorized key(s)\n",
        plan.cfg.ssh_keys.len()
    ));
    s.push_str(&format!("Services: {}\n", plan.cfg.services.join(", ")));
    s
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

    let plan = plan(cfg)?;
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

/// True when `path` is a mount point.
fn is_mounted(path: &Path) -> bool {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return false;
    };
    let want = path.to_string_lossy().replace(' ', "\\040");
    for line in mounts.lines() {
        let mut f = line.splitn(3, ' ');
        f.next();
        if f.next() == Some(&want) {
            return true;
        }
    }
    false
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
    let _ = std::process::Command::new("partprobe")
        .arg(target.active_device())
        .output();

    // 2. partition devices (slot B is looked up for the record only)
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

    // 3. whole-image deployment
    deploy::deploy(&plan.artifacts, &parts, log)?;

    // 4. mounts for content initialization
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
    let esp_m = target.mount(&esp_dev, "vfat", work, "")?;
    let slot_m = target.mount(&slot_dev, "erofs", work, "ro")?;

    // 5. /var/lib/etc from factory defaults + config
    etcinit::run(&plan.cfg, &var_m, &home_m, &slot_m, log)?;

    // 6. ESP / UKI verification
    let uki = plan
        .artifacts
        .iter()
        .find(|a| a.kind == "efi")
        .expect("artifact set always contains the efi");
    boot::verify(&esp_m, uki, v, log)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg() -> Config {
        crate::config::parse(
            r#"
schema = 1
[target]
disk = "/tmp/does-not-matter-for-report"
[source]
base = "/tmp/does-not-matter"
version = "0.1.0"
[system]
hostname = "ingot1"
timezone = "UTC"
locale = "C.UTF-8"
keymap = "us"
[partitions]
esp = "1G"
slot_a = "8G"
slot_b = "8G"
var = "4G"
home = "8G"
[filesystems]
slot = "erofs"
var = "btrfs"
home = "btrfs"
[encryption]
var = "none"
home = "none"
[[users]]
name = "tommy"
shell = "/usr/bin/brush"
[ssh]
authorized_keys = ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= tommy@ingot"]
[services]
enabled = ["sshd.service"]
"#,
        )
        .expect("sample config parses")
    }

    #[test]
    fn report_covers_partitions_and_deploys() {
        let cfg = sample_cfg();
        let layout = layout::compute(&cfg);
        let artifacts = vec![
            Artifact {
                kind: "slot.raw",
                path: PathBuf::from("/x/ingot_0.1.0.slot.raw"),
                size: 322_000_000,
            },
            Artifact {
                kind: "efi",
                path: PathBuf::from("/x/ingot_0.1.0.efi"),
                size: 21_000_000,
            },
            Artifact {
                kind: "esp.raw",
                path: PathBuf::from("/x/ingot_0.1.0.esp.raw"),
                size: 220_000_000,
            },
            Artifact {
                kind: "var.raw",
                path: PathBuf::from("/x/ingot_0.1.0.var.raw"),
                size: 4_800_000,
            },
            Artifact {
                kind: "home.raw",
                path: PathBuf::from("/x/ingot_0.1.0.home.raw"),
                size: 4_800_000,
            },
        ];
        let plan = Plan {
            cfg,
            layout,
            artifacts,
            checksums: vec![None, None, None, None, None],
            disk_bytes: 29 * (1 << 30),
            disk_format: "qcow2".to_string(),
        };
        let report = render_report(&plan);
        for needle in [
            "ingot_0.1.0.slot.raw",
            "slot B stays _empty",
            "f357e520-642b-5b9b-abc7-9e5969de5691",
            "0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
            "59f1269b-a18e-558b-a225-8394c1d1bc0f",
            "501347aa-775a-5736-8da3-2a9977c820ec",
            "9e772d30-44c4-5784-8be1-0832e3c1c4db",
            "tommy (uid 1000, /usr/bin/brush)",
            "sshd.service",
            "hostname=ingot1",
            "29 GiB",
        ] {
            assert!(
                report.contains(needle),
                "report missing {needle:?}\n{report}"
            );
        }
    }

    #[test]
    fn plan_rejects_encrypted_state_in_v1() {
        let cfg = sample_cfg();
        let mut cfg = cfg;
        cfg.enc_var = crate::config::Encryption::Luks2;
        let e = plan(cfg).unwrap_err();
        assert!(e.contains("luks2"), "{e}");
    }

    #[test]
    fn plan_rejects_missing_artifacts() {
        // plan() validates the disk before the artifacts: give it a
        // real (sparse) raw disk so the artifact check is reached.
        let dir = std::env::temp_dir().join(format!("ingot-plan-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let disk = dir.join("target.raw");
        let f = fs::File::create(&disk).unwrap();
        f.set_len(30 * (1 << 30)).unwrap();
        drop(f);

        let mut cfg = sample_cfg();
        cfg.target_disk = disk.to_string_lossy().to_string();
        let e = plan(cfg).unwrap_err();
        assert!(e.contains("artifact not found"), "{e}");
        let _ = fs::remove_dir_all(&dir);
    }
}
