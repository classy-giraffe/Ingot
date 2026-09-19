//! Step forms (Steps 1-9) for the Ingot Installer TUI.

use super::{Step, Ui};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const LABEL_W: usize = 18;

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
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

/// Helper returning (label, value) rows for single-field steps.
pub fn field_rows(ui: &Ui) -> Vec<(&'static str, String)> {
    match ui.step {
        Step::Target => vec![("Target Disk", ui.draft.target_disk.clone())],
        Step::Source => vec![
            ("Source Base", ui.draft.source_base.clone()),
            ("Release Version", ui.draft.version.clone()),
        ],
        Step::System => vec![
            ("Hostname", ui.draft.hostname.clone()),
            ("Timezone", ui.draft.timezone.clone()),
            ("System Locale", ui.draft.locale.clone()),
            ("Keyboard Map", ui.draft.keymap.clone()),
        ],
        Step::Partitions => vec![
            ("ESP (Boot)", ui.draft.esp.clone()),
            ("Slot A (OS)", ui.draft.slot_a.clone()),
            ("Slot B (OS)", ui.draft.slot_b.clone()),
            ("Var (/var)", ui.draft.var.clone()),
            ("Home (/home)", ui.draft.home.clone()),
        ],
        _ => Vec::new(),
    }
}

/// Renders steps 1-9.
pub fn step_lines(
    ui: &Ui,
    w: usize,
    out: &mut Vec<Line<'static>>,
    cursor: &mut Option<(u16, u16)>,
) {
    match ui.step {
        Step::Target => render_target(ui, w, out, cursor),
        Step::Source => render_source(ui, w, out, cursor),
        Step::System | Step::Partitions => render_fields(ui, w, out, cursor),
        Step::Filesystems => render_filesystems(ui, out),
        Step::Encryption => render_encryption(ui, out),
        Step::Users => render_users(ui, w, out, cursor),
        Step::Ssh | Step::Services => render_list(ui, w, out, cursor),
        Step::Review => {}
    }
}

fn render_target(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>, cursor: &mut Option<(u16, u16)>) {
    out.push(Line::from(vec![
        Span::styled(" Select the installation target disk. ", bold().fg(Color::White)),
        Span::styled("ALL EXISTING DATA WILL BE ERASED.", bold().fg(Color::Red)),
    ]));
    out.push(Line::default());

    let val = &ui.draft.target_disk;
    let val_trunc: String = val.chars().take(w.saturating_sub(LABEL_W + 8)).collect();
    out.push(Line::from(vec![
        Span::styled(format!("  {:<LABEL_W$} ", "Target Device:"), bold().fg(Color::Cyan)),
        Span::styled(format!("[ {val_trunc} ]"), bold().fg(Color::White)),
    ]));
    *cursor = Some(((LABEL_W + 5 + val_trunc.len()) as u16, 2));

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  Available Block Devices ", bold().fg(Color::Yellow)),
        Span::styled("(use [Left]/[Right] or [Up]/[Down] to select):", dim()),
    ]));

    let disks = crate::target::probe_disks();
    if disks.is_empty() {
        out.push(Line::from(Span::styled("    (no candidate block devices found in /sys/block)", dim())));
    } else {
        for d in &disks {
            let is_sel = d.path.to_string_lossy() == ui.draft.target_disk;
            let (pointer, p_st) = if is_sel {
                ("  ► ● ", bold().fg(Color::Green))
            } else {
                ("    ○ ", dim())
            };
            let role = if d.read_only {
                format!("[read-only media: {}]", if d.model.is_empty() { "ISO" } else { &d.model })
            } else if is_sel {
                "[INSTALLATION TARGET]".into()
            } else {
                "[writable disk]".into()
            };
            out.push(Line::from(vec![
                Span::styled(pointer, p_st),
                Span::styled(format!("{:<12} ", d.path.display()), if is_sel { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) }),
                Span::styled(format!("{:>8}  ", d.size_human()), bold().fg(Color::Cyan)),
                Span::styled(role, if is_sel { bold().fg(Color::Green) } else { dim() }),
            ]));
        }
    }
}

fn render_source(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>, cursor: &mut Option<(u16, u16)>) {
    let is_live = std::path::Path::new(&ui.draft.source_base)
        .join(crate::source::LIVE_EROFS)
        .is_file();

    if is_live {
        out.push(Line::from(vec![
            Span::styled("  ● ", bold().fg(Color::Green)),
            Span::styled("Live ISO media detected at ", bold().fg(Color::Green)),
            Span::styled(ui.draft.source_base.clone(), bold().fg(Color::White)),
        ]));
        out.push(Line::from(Span::styled(
            "    The running OS payload will be deployed directly to the target slots.",
            Style::default().fg(Color::Gray),
        )));
    } else {
        out.push(Line::from(Span::styled(
            " Specify the base directory holding release artifacts (slot.raw, UKI, ESP).",
            Style::default().fg(Color::White),
        )));
    }
    out.push(Line::default());

    for (i, (label, value)) in field_rows(ui).into_iter().enumerate() {
        let active = i == ui.sel;
        let val_trunc: String = value.chars().take(w.saturating_sub(LABEL_W + 8)).collect();
        let indicator = if active { "► " } else { "  " };
        let l_st = if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) };
        let v_st = if active { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("{:<LABEL_W$} ", format!("{label}:")), l_st),
            Span::styled(format!("[ {val_trunc} ]"), v_st),
        ]));
        if active {
            *cursor = Some(((LABEL_W + 5 + val_trunc.len()) as u16, (out.len() - 1) as u16));
        }
    }
}

fn render_fields(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>, cursor: &mut Option<(u16, u16)>) {
    let guide = match ui.step {
        Step::System => " Configure machine hostname, timezone, locale, and keymap.",
        Step::Partitions => " Allocate partition sizes in binary units (e.g. 512M, 1G, 8G). Multiples of 1 MiB.",
        _ => "",
    };
    out.push(Line::from(Span::styled(guide, Style::default().fg(Color::Gray))));
    out.push(Line::default());

    for (i, (label, value)) in field_rows(ui).into_iter().enumerate() {
        let active = i == ui.sel;
        let val_trunc: String = value.chars().take(w.saturating_sub(LABEL_W + 8)).collect();
        let indicator = if active { "► " } else { "  " };
        let l_st = if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) };
        let v_st = if active { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("{:<LABEL_W$} ", format!("{label}:")), l_st),
            Span::styled(format!("[ {val_trunc} ]"), v_st),
        ]));
        if active {
            *cursor = Some(((LABEL_W + 5 + val_trunc.len()) as u16, (out.len() - 1) as u16));
        }
    }
}

fn render_filesystems(ui: &Ui, out: &mut Vec<Line<'static>>) {
    out.push(Line::from(Span::styled(
        " Choose filesystem types for state partitions. OS slots use read-only erofs.",
        Style::default().fg(Color::Gray),
    )));
    out.push(Line::default());

    let rows = [
        ("Slot Partition:", "erofs", "(fixed: immutable A/B payload)"),
        ("Var Partition (/var):", fs_name(ui.draft.fs_var), "(use [Left]/[Right] to toggle: btrfs / ext4)"),
        ("Home Partition (/home):", fs_name(ui.draft.fs_home), "(use [Left]/[Right] to toggle: btrfs / ext4)"),
    ];

    for (i, (label, value, note)) in rows.into_iter().enumerate() {
        let active = i == ui.sel;
        let indicator = if active { "► " } else { "  " };
        let l_st = if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("{:<24} ", label), l_st),
            Span::styled(format!("[ {value:<6} ]"), if active { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) }),
            Span::styled(format!("  {note}"), if active { yellow() } else { dim() }),
        ]));
    }
}

fn render_encryption(ui: &Ui, out: &mut Vec<Line<'static>>) {
    out.push(Line::from(Span::styled(
        " Configure storage encryption. (Note: v1 workstation baseline is unencrypted).",
        Style::default().fg(Color::Gray),
    )));
    out.push(Line::default());

    let rows = [
        ("Var Partition (/var):", enc_name(ui.draft.enc_var)),
        ("Home Partition (/home):", enc_name(ui.draft.enc_home)),
    ];

    for (i, (label, value)) in rows.into_iter().enumerate() {
        let active = i == ui.sel;
        let indicator = if active { "► " } else { "  " };
        let note = if value == "luks2" {
            "luks2 is not supported in v1 (engine will reject)"
        } else {
            "none (standard unencrypted v1)"
        };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("{:<24} ", label), if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) }),
            Span::styled(format!("[ {value:<6} ]"), if active { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) }),
            Span::styled(format!("  ({note})"), if value == "luks2" { Style::default().fg(Color::Red) } else { dim() }),
        ]));
    }
}

fn render_users(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>, cursor: &mut Option<(u16, u16)>) {
    out.push(Line::from(Span::styled(
        " Configure initial administrator accounts. Created with passwordless sudo.",
        Style::default().fg(Color::Gray),
    )));
    out.push(Line::default());

    if ui.draft.users.is_empty() {
        out.push(Line::from(Span::styled("  (no users configured: press [Insert] to add an admin user)", yellow())));
    }
    for (i, u) in ui.draft.users.iter().enumerate() {
        let active = i == ui.sel;
        let name_val: String = u.name.chars().take(w.saturating_sub(40)).collect();
        let shell_val: String = if u.shell.is_empty() {
            "default (nushell)".to_string()
        } else {
            u.shell.chars().take(w.saturating_sub(40)).collect()
        };

        let indicator = if active { "► " } else { "  " };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("User {}:  Username: ", i + 1), if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) }),
            Span::styled(format!("[ {name_val:<12} ]"), if active && ui.sub == 0 { bold().fg(Color::White).add_modifier(Modifier::UNDERLINED) } else { Style::default().fg(Color::Gray) }),
            Span::styled("  Shell: ", if active { bold().fg(Color::Cyan) } else { Style::default().fg(Color::Gray) }),
            Span::styled(format!("[ {shell_val:<18} ]"), if active && ui.sub == 1 { bold().fg(Color::White).add_modifier(Modifier::UNDERLINED) } else { Style::default().fg(Color::Gray) }),
        ]));
        if active {
            let col = if ui.sub == 0 {
                2 + 19 + 2 + name_val.len()
            } else {
                2 + 19 + 2 + 12 + 2 + 9 + 2 + shell_val.len()
            };
            *cursor = Some((col.min(w.saturating_sub(2)) as u16, (out.len() - 1) as u16));
        }
    }
}

fn render_list(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>, cursor: &mut Option<(u16, u16)>) {
    let (title, items) = match ui.step {
        Step::Ssh => (
            " Authorized SSH public keys (placed in initial user's ~/.ssh/authorized_keys).",
            &ui.draft.ssh_keys,
        ),
        _ => (
            " Additional systemd service units to enable at first boot.",
            &ui.draft.services,
        ),
    };
    out.push(Line::from(Span::styled(title, Style::default().fg(Color::Gray))));
    out.push(Line::default());

    if items.is_empty() {
        out.push(Line::from(Span::styled("  (empty list: press [Insert] to add an entry, or [Enter] to proceed)", dim())));
    }
    for (i, item) in items.iter().enumerate() {
        let active = i == ui.sel;
        let val_trunc: String = item.chars().take(w.saturating_sub(16)).collect();
        let indicator = if active { "► " } else { "  " };
        out.push(Line::from(vec![
            Span::styled(indicator, if active { bold().fg(Color::Cyan) } else { dim() }),
            Span::styled(format!("{:>2}. ", i + 1), dim()),
            Span::styled(format!("[ {val_trunc} ]"), if active { bold().fg(Color::White) } else { Style::default().fg(Color::Gray) }),
        ]));
        if active {
            *cursor = Some(((4 + 4 + 2 + val_trunc.len()).min(w.saturating_sub(2)) as u16, (out.len() - 1) as u16));
        }
    }
}
