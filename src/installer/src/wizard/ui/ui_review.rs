//! Review, Plan, Confirmation, and Done screens for the Ingot Installer TUI.

use super::Ui;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn red() -> Style {
    Style::default().fg(Color::Red)
}

fn green() -> Style {
    Style::default().fg(Color::Green)
}

fn cyan() -> Style {
    Style::default().fg(Color::Cyan)
}

fn yellow() -> Style {
    Style::default().fg(Color::Yellow)
}

fn fs_name(fs: crate::config::StateFs) -> &'static str {
    match fs {
        crate::config::StateFs::Btrfs => "btrfs",
        crate::config::StateFs::Ext4 => "ext4",
    }
}

fn enc_name(e: crate::config::Encryption) -> &'static str {
    match e {
        crate::config::Encryption::None => "none",
        crate::config::Encryption::Luks2 => "luks2",
    }
}

/// Renders the executive summary for Step 10 (Review).
pub fn review_lines(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>) {
    let d = &ui.draft;

    out.push(Line::from(vec![
        Span::styled(" ╭─ ", dim()),
        Span::styled("Target Storage & Source Media", bold().fg(Color::Cyan)),
        Span::styled(" ───────────────────────────────────────────────────", dim()),
    ]));

    let target_label = if d.target_disk.is_empty() {
        "(none selected)".to_string()
    } else {
        format!("{} (ALL DATA WILL BE OVERWRITTEN)", d.target_disk)
    };
    out.push(Line::from(vec![
        Span::styled(" │  Target Device:      ", bold().fg(Color::White)),
        Span::styled(target_label, bold().fg(Color::Yellow)),
    ]));

    let is_live = std::path::Path::new(&d.source_base)
        .join(crate::source::LIVE_EROFS)
        .is_file();
    let source_desc = if is_live {
        format!("Live ISO media ({}) [running release]", d.source_base)
    } else {
        format!("Directory artifacts ({} / version {})", d.source_base, d.version)
    };
    out.push(Line::from(vec![
        Span::styled(" │  Source Image:       ", bold().fg(Color::White)),
        Span::styled(source_desc, Style::default().fg(Color::White)),
    ]));

    let part_summary = format!(
        "ESP: {} | Slot A: {} | Slot B: {} | Var: {} | Home: {}",
        d.esp, d.slot_a, d.slot_b, d.var, d.home
    );
    out.push(Line::from(vec![
        Span::styled(" │  Partitions:         ", bold().fg(Color::White)),
        Span::styled(part_summary, Style::default().fg(Color::Gray)),
    ]));

    let fs_summary = format!(
        "Slots: erofs (immutable A/B) | State (/var, /home): {}",
        fs_name(d.fs_var)
    );
    out.push(Line::from(vec![
        Span::styled(" │  Filesystems:        ", bold().fg(Color::White)),
        Span::styled(fs_summary, Style::default().fg(Color::Gray)),
    ]));
    out.push(Line::from(Span::styled(" ╰──────────────────────────────────────────────────────────────────────────", dim())));

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled(" ╭─ ", dim()),
        Span::styled("System Identity & Security", bold().fg(Color::Cyan)),
        Span::styled(" ───────────────────────────────────────────────────────", dim()),
    ]));
    out.push(Line::from(vec![
        Span::styled(" │  Hostname:           ", bold().fg(Color::White)),
        Span::styled(d.hostname.clone(), bold().fg(Color::White)),
    ]));
    let sys_loc = format!(
        "Timezone: {}  |  Locale: {}  |  Keymap: {}",
        d.timezone, d.locale, d.keymap
    );
    out.push(Line::from(vec![
        Span::styled(" │  Localization:       ", bold().fg(Color::White)),
        Span::styled(sys_loc, Style::default().fg(Color::Gray)),
    ]));
    let enc_desc = if d.enc_var == crate::config::Encryption::None && d.enc_home == crate::config::Encryption::None {
        "None (standard unencrypted v1 baseline)".to_string()
    } else {
        format!("Var: {}  |  Home: {}", enc_name(d.enc_var), enc_name(d.enc_home))
    };
    out.push(Line::from(vec![
        Span::styled(" │  Encryption:         ", bold().fg(Color::White)),
        Span::styled(enc_desc, Style::default().fg(Color::Gray)),
    ]));
    out.push(Line::from(Span::styled(" ╰──────────────────────────────────────────────────────────────────────────", dim())));

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled(" ╭─ ", dim()),
        Span::styled("Administrator Accounts & Access", bold().fg(Color::Cyan)),
        Span::styled(" ──────────────────────────────────────────", dim()),
    ]));
    let users_desc = if d.users.is_empty() {
        "(none configured)".to_string()
    } else {
        d.users
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let sh = if u.shell.is_empty() { "default (nushell)" } else { &u.shell };
                format!("{} (UID {}, shell: {})", u.name, 1000 + i, sh)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    out.push(Line::from(vec![
        Span::styled(" │  Users:              ", bold().fg(Color::White)),
        Span::styled(users_desc, Style::default().fg(Color::White)),
    ]));
    let ssh_desc = if d.ssh_keys.is_empty() {
        "0 public keys configured (console / password login)".to_string()
    } else {
        format!("{} authorized public key(s) installed", d.ssh_keys.len())
    };
    out.push(Line::from(vec![
        Span::styled(" │  SSH Access:         ", bold().fg(Color::White)),
        Span::styled(ssh_desc, Style::default().fg(Color::Gray)),
    ]));
    let srv_desc = if d.services.is_empty() {
        "Standard core baseline (sshd, networkd, getty, journald)".to_string()
    } else {
        d.services.join(", ")
    };
    out.push(Line::from(vec![
        Span::styled(" │  Custom Services:    ", bold().fg(Color::White)),
        Span::styled(srv_desc, Style::default().fg(Color::Gray)),
    ]));
    out.push(Line::from(Span::styled(" ╰──────────────────────────────────────────────────────────────────────────", dim())));

    out.push(Line::default());
    if ui.errors.is_empty() {
        out.push(Line::from(vec![
            Span::styled("  ✓ ", bold().fg(Color::Green)),
            Span::styled("Configuration validated cleanly. ", bold().fg(Color::Green)),
            Span::styled("Press [Enter] to generate partition plan and proceed.", Style::default().fg(Color::White)),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::styled("  ✖ ", bold().fg(Color::Red)),
            Span::styled("Validation diagnostics found: ", bold().fg(Color::Red)),
            Span::styled("Press [Esc] to navigate back and correct settings.", Style::default().fg(Color::Yellow)),
        ]));
        for e in &ui.errors {
            let e_trunc: String = e.chars().take(w.saturating_sub(8)).collect();
            out.push(Line::from(Span::styled(format!("    • {e_trunc}"), red())));
        }
    }
}

/// Renders the engine's partition plan report.
pub fn plan_lines(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>) {
    out.push(Line::default());
    match &ui.plan {
        Ok(report) => {
            out.push(Line::from(vec![
                Span::styled("  ✓ ", bold().fg(Color::Green)),
                Span::styled("Partition Plan Computed Successfully", bold().fg(Color::Green)),
                Span::styled(format!("  (config: {})", ui.config_out.display()), dim()),
            ]));
            out.push(Line::default());
            for l in report.lines() {
                let l_trunc: String = l.chars().take(w.saturating_sub(4)).collect();
                out.push(Line::from(Span::styled(format!("  {l_trunc}"), Style::default().fg(Color::White))));
            }
            if ui.dry_run {
                out.push(Line::default());
                out.push(Line::from(Span::styled(
                    "  [DRY RUN MODE] The target disk will NOT be modified.",
                    yellow().add_modifier(Modifier::BOLD),
                )));
            }
        }
        Err(e) => {
            out.push(Line::from(vec![
                Span::styled("  ✖ ", bold().fg(Color::Red)),
                Span::styled("Plan Generation Failed:", bold().fg(Color::Red)),
            ]));
            out.push(Line::default());
            out.push(Line::from(Span::styled(format!("    {e}"), red())));
        }
    }
}

/// Renders the confirmation prompt before destructive write.
pub fn confirm_lines(
    ui: &Ui,
    w: usize,
    out: &mut Vec<Line<'static>>,
    cursor: &mut Option<(u16, u16)>,
) {
    out.push(Line::default());
    let target: String = ui.draft.target_disk.chars().take(w.saturating_sub(30)).collect();

    out.push(Line::from(Span::styled(" ╭─ DESTRUCTIVE ACTION WARNING ─────────────────────────────────────────────╮", red().add_modifier(Modifier::BOLD))));
    out.push(Line::from(vec![
        Span::styled(" │  ", red().add_modifier(Modifier::BOLD)),
        Span::styled(format!("About to partition and format disk: {target}"), bold().fg(Color::White)),
    ]));
    out.push(Line::from(Span::styled(" │  ALL EXISTING DATA, PARTITIONS, AND FILESYSTEMS WILL BE DESTROYED.       │", red().add_modifier(Modifier::BOLD))));
    out.push(Line::from(Span::styled(" │  This operation cannot be undone.                                        │", red())));
    out.push(Line::from(Span::styled(" ╰──────────────────────────────────────────────────────────────────────────╯", red().add_modifier(Modifier::BOLD))));

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  Configuration file: ", bold()),
        Span::styled(ui.config_out.display().to_string(), cyan()),
    ]));
    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  Type ", Style::default()),
        Span::styled("yes", bold().fg(Color::Yellow)),
        Span::styled(" and press [Enter] to proceed with installation: ", Style::default()),
        Span::styled(ui.confirm.clone(), bold().fg(Color::White).add_modifier(Modifier::UNDERLINED)),
    ]));

    let prompt_len = 2 + 5 + 3 + 49 + ui.confirm.len();
    *cursor = Some((prompt_len.min(w.saturating_sub(2)) as u16, 8));
}

/// Renders the post-installation completion screen.
pub fn done_lines(ui: &Ui, out: &mut Vec<Line<'static>>) {
    out.push(Line::default());
    if ui.dry_run {
        out.push(Line::from(Span::styled(
            "  ✓ Dry run complete - no disks were modified.",
            green().add_modifier(Modifier::BOLD),
        )));
    } else {
        out.push(Line::from(Span::styled(" ╭─ INSTALLATION COMPLETE ───────────────────────────────────────────────────╮", green().add_modifier(Modifier::BOLD))));
        out.push(Line::from(Span::styled(" │  Ingot OS has been successfully installed to the target disk.             │", bold().fg(Color::White))));
        out.push(Line::from(Span::styled(" │  A/B dual slots deployed, UKI signed for Secure Boot, ESP configured.     │", Style::default().fg(Color::Gray))));
        out.push(Line::from(Span::styled(" ╰──────────────────────────────────────────────────────────────────────────╯", green().add_modifier(Modifier::BOLD))));
    }
    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  Installed Config:  ", bold()),
        Span::styled(ui.config_out.display().to_string(), cyan()),
    ]));
    out.push(Line::from(vec![
        Span::styled("  Installation Log:  ", bold()),
        Span::styled(format!("{} (persisted to target /var/lib/ingot/install.log)", ui.log_name()), Style::default().fg(Color::Gray)),
    ]));
    out.push(Line::default());
    out.push(Line::from(Span::styled(
        "  You may now reboot into the installed system: run 'systemctl reboot'.",
        bold().fg(Color::Cyan),
    )));
}
