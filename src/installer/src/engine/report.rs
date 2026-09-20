//! Formatting and reporting of the dry-run install plan.

use super::{Plan, PlanSource};
use crate::config::Config;
use crate::size::human;

/// The dry-run report (stdout), covering partitions, sizes, and deployments.
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
    match &plan.source {
        PlanSource::Artifacts { .. } => {
            s.push_str("Deployment:\n");
            for (kind, part, what) in [
                ("slot.raw", "slot A", "active version image"),
                ("esp.raw", "esp", "systemd-boot + UKI"),
                ("var.raw", "var", "factory state"),
                ("home.raw", "home", "factory state"),
            ] {
                let art = plan.source.artifact(kind).unwrap();
                s.push_str(&format!(
                    "  ingot_{v}.{kind} ({} bytes) -> {part} ({what})\n",
                    human(art.size)
                ));
            }
            s.push_str("  slot B stays _empty (unformatted; an update fills it)\n\n");
            s.push_str("Artifacts (sha256 from .sha256 sidecars when present):\n");
            for (a, sum) in plan.source.artifacts().iter().zip(plan.source.checksums()) {
                let sum = sum.clone().unwrap_or_else(|| "- (no sidecar)".to_string());
                s.push_str(&format!("  ingot_{v}.{}  {sum}\n", a.kind));
            }
        }
        PlanSource::Live(live) => {
            s.push_str("Deployment (live source: the running release on the live ISO):\n");
            let erofs_size = live
                .erofs
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0);
            s.push_str(&format!(
                "  {} ({} bytes) -> slot A (the running release's payload)\n",
                live.erofs.display(),
                human(erofs_size)
            ));
            s.push_str(&format!(
                "  {} (ESP tree: systemd-boot + UKI + loader) -> esp\n",
                live.esp_tree.display()
            ));
            s.push_str("  var, home: formatted in place (repart); factory state is empty\n");
            s.push_str("  slot B stays _empty (unformatted; an update fills it)\n\n");
            s.push_str("Live source (sha256 of the media's files):\n");
            s.push_str(&format!("  LiveOS/rootfs.erofs  {}\n", live.erofs_sha));
            s.push_str(&format!(
                "  esp/EFI/Linux/ingot_{v}.efi  {}\n",
                live.uki_sha
            ));
        }
    }
    s.push_str(&format!(
        "\nUKI ingot_{v}.efi: valid (VERSION_ID {v}, fixed PARTUUIDs in command line)\n"
    ));
    s.push_str(&format!(
        "System: hostname={} timezone={} locale={} keymap={}\n",
        plan.cfg.hostname, plan.cfg.timezone, plan.cfg.locale, plan.cfg.keymap
    ));
    s.push_str(&system_lines(&plan.cfg));
    s
}

/// The Users, SSH keys, and Services lines of the dry-run report.
fn system_lines(cfg: &Config) -> String {
    let users: Vec<String> = cfg
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
    let mut s = String::new();
    s.push_str(&format!("Users: {}\n", users.join(", ")));
    s.push_str(&format!(
        "SSH keys: {} authorized key(s)\n",
        cfg.ssh_keys.len()
    ));
    s.push_str(&format!("Services: {}\n", cfg.services.join(", ")));
    s
}
