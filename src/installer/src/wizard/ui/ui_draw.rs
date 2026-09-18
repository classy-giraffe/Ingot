//! Rendering for the wizard screens: a line-based layout (one line
//! per field/row) inside a bordered content block, a footer hint
//! bar on the bottom line, diagnostics in red, and the terminal
//! cursor on the active field.

use super::{Screen, Ui};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::DefaultTerminal;
use std::io;

const LABEL_W: usize = 11;

/// Draws one frame of the wizard.
pub fn draw(terminal: &mut DefaultTerminal, ui: &mut Ui) -> io::Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let areas =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
        let (content, footer) = (areas[0], areas[1]);
        let (lines, cursor) = render_lines(ui, content.width as usize);
        let para = Paragraph::new(lines).block(Block::bordered().title(Line::from(format!(
            " Ingot installer - wizard: {}",
            screen_title(ui)
        ))));
        f.render_widget(para, content);
        let hint = ui.flash.clone().unwrap_or_else(|| footer_hint(ui).to_string());
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {hint}"), yellow()))),
            footer,
        );
        if let Some((x, y)) = cursor {
            f.set_cursor_position((
                x.min(content.width.saturating_sub(1)),
                y.min(content.height.saturating_sub(1)),
            ));
        }
    })
    .map(|_| ())
}

fn screen_title(ui: &Ui) -> String {
    match ui.screen {
        Screen::Step => format!("{} ({} of 10)", ui.step.title(), ui.step.index() + 1),
        Screen::Plan => "plan (from the written config)".into(),
        Screen::Confirm => "confirm".into(),
        Screen::Done => "done".into(),
    }
}

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

fn yellow() -> Style {
    Style::default().fg(Color::Yellow)
}

/// Builds the content lines plus the cursor position (content-chunk
/// relative), if any.
fn render_lines(ui: &Ui, w: usize) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cursor: Option<(u16, u16)> = None;
    match ui.screen {
        Screen::Step => step_lines(ui, w, &mut out, &mut cursor),
        Screen::Plan => plan_lines(ui, w, &mut out),
        Screen::Confirm => confirm_lines(ui, w, &mut out, &mut cursor),
        Screen::Done => done_lines(ui, &mut out),
    }
    if let Some(p) = &ui.prompt {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            format!(" {p}"),
            red().add_modifier(Modifier::BOLD),
        )));
        cursor = None;
    }
    (out, cursor)
}

fn footer_hint(ui: &Ui) -> &'static str {
    use super::Step;
    match ui.screen {
        Screen::Step => match ui.step {
            Step::Users => " up/down user - left/right field - insert add - delete remove - enter next - esc back - ctrl-c abort",
            Step::Ssh | Step::Services => " up/down move - insert add - delete remove - enter next - esc back - ctrl-c abort",
            Step::Review => " enter write config + show plan - esc edit",
            _ => " up/down move - enter next - backtab/esc back - ctrl-c abort",
        },
        Screen::Plan => {
            if ui.dry_run {
                " enter finish (dry run: no disk is touched)"
            } else if ui.plan.is_err() {
                " plan failed: esc back to edit"
            } else {
                " enter to the confirmation - esc back"
            }
        }
        Screen::Confirm => " type 'yes' + enter to install - esc abort",
        Screen::Done => " enter to exit",
    }
}

/// Content lines for the step screens. The first line is blank, so
/// a row at index i sits at content-chunk y = i + 1.
fn step_lines(
    ui: &Ui,
    w: usize,
    out: &mut Vec<Line<'static>>,
    cursor: &mut Option<(u16, u16)>,
) {
    use super::Step;
    out.push(Line::default());
    match ui.step {
        Step::Target | Step::Source | Step::System | Step::Partitions => {
            for (i, (label, value)) in field_rows(ui).into_iter().enumerate() {
                let active = i == ui.sel;
                let value: String = value.chars().take(w.saturating_sub(LABEL_W + 7)).collect();
                out.push(Line::from(vec![
                    Span::raw(format!("  {:<LABEL_W$} ", label)),
                    Span::styled(value.clone(), if active { bold() } else { Style::default() }),
                ]));
                if active {
                    *cursor = Some(((LABEL_W + 4 + value.len()) as u16, i as u16 + 1));
                }
            }
        }
        Step::Filesystems => {
            for (i, (label, value, note)) in [
                ("slot", "erofs", "fixed in v1"),
                ("var", fs_name(ui.draft.fs_var), "left/right to change"),
                ("home", fs_name(ui.draft.fs_home), "left/right to change"),
            ]
            .into_iter()
            .enumerate()
            {
                out.push(Line::from(vec![
                    Span::raw(format!("  {:<LABEL_W$} ", label)),
                    Span::styled(
                        value.to_string(),
                        if i == ui.sel { bold() } else { Style::default() },
                    ),
                    Span::styled(format!("  ({note})"), dim()),
                ]));
            }
        }
        Step::Encryption => {
            for (i, (label, value)) in [
                ("var", enc_name(ui.draft.enc_var)),
                ("home", enc_name(ui.draft.enc_home)),
            ]
            .into_iter()
            .enumerate()
            {
                let note = if value == "luks2" {
                    "left/right to change - rejected by the v1 engine"
                } else {
                    "left/right to change"
                };
                out.push(Line::from(vec![
                    Span::raw(format!("  {:<LABEL_W$} ", label)),
                    Span::styled(
                        value.to_string(),
                        if i == ui.sel { bold() } else { Style::default() },
                    ),
                    Span::styled(format!("  ({note})"), dim()),
                ]));
            }
        }
        Step::Users => {
            if ui.draft.users.is_empty() {
                out.push(Line::from(Span::styled("  (no users: press insert to add one)", dim())));
            }
            for (i, u) in ui.draft.users.iter().enumerate() {
                let active = i == ui.sel;
                let name_style = if active && ui.sub == 0 {
                    bold().add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default()
                };
                let shell_style = if active && ui.sub == 1 {
                    bold().add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default()
                };
                let name: String = u.name.chars().take(w.saturating_sub(40)).collect();
                let shell: String = if u.shell.is_empty() {
                    "(default)".to_string()
                } else {
                    u.shell.chars().take(w.saturating_sub(40)).collect()
                };
                out.push(Line::from(vec![
                    Span::raw(format!("  {} {}. name: ", if active { '>' } else { ' ' }, i + 1)),
                    Span::styled(name.clone(), name_style),
                    Span::raw("  shell: "),
                    Span::styled(shell.clone(), shell_style),
                ]));
                if active {
                    let col = if ui.sub == 0 {
                        4 + 6 + name.len()
                    } else {
                        4 + 6 + name.len() + 9 + shell.len()
                    };
                    *cursor = Some((col as u16, i as u16 + 1));
                }
            }
        }
        Step::Ssh | Step::Services => {
            let items: &Vec<String> = match ui.step {
                Step::Ssh => &ui.draft.ssh_keys,
                _ => &ui.draft.services,
            };
            if items.is_empty() {
                out.push(Line::from(Span::styled(
                    "  (empty: press insert to add a line)",
                    dim(),
                )));
            }
            for (i, item) in items.iter().enumerate() {
                let active = i == ui.sel;
                let shown: String = item.chars().take(w.saturating_sub(7)).collect();
                out.push(Line::from(vec![
                    Span::raw(format!("  {i} ")),
                    Span::styled(shown.clone(), if active { bold() } else { Style::default() }),
                ]));
                if active {
                    *cursor = Some(((4 + shown.len()) as u16, i as u16 + 1));
                }
            }
        }
        Step::Review => review_lines(ui, w, out),
    }
    // Step diagnostics (the engine's, filtered to this category).
    let errs = ui.step_errors();
    if !errs.is_empty() {
        out.push(Line::default());
        for e in &errs {
            out.push(Line::from(Span::styled(format!("  ! {e}"), red())));
        }
    }
}

/// The (label, value) rows of the single-field steps.
fn field_rows(ui: &Ui) -> Vec<(&'static str, String)> {
    use super::Step;
    match ui.step {
        Step::Target => vec![("disk", ui.draft.target_disk.clone())],
        Step::Source => vec![
            ("base", ui.draft.source_base.clone()),
            ("version", ui.draft.version.clone()),
        ],
        Step::System => vec![
            ("hostname", ui.draft.hostname.clone()),
            ("timezone", ui.draft.timezone.clone()),
            ("locale", ui.draft.locale.clone()),
            ("keymap", ui.draft.keymap.clone()),
        ],
        Step::Partitions => vec![
            ("esp", ui.draft.esp.clone()),
            ("slot_a", ui.draft.slot_a.clone()),
            ("slot_b", ui.draft.slot_b.clone()),
            ("var", ui.draft.var.clone()),
            ("home", ui.draft.home.clone()),
        ],
        _ => Vec::new(),
    }
}

fn review_lines(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>) {
    let d = &ui.draft;
    let users: Vec<String> = d
        .users
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let shell = if u.shell.is_empty() {
                "default shell".to_string()
            } else {
                u.shell.clone()
            };
            format!("{} (uid {}, {})", u.name, 1000 + i as u32, shell)
        })
        .collect();
    let rows: Vec<(&str, String)> = vec![
        ("target", d.target_disk.clone()),
        ("source", format!("{} ({})", d.source_base, d.version)),
        (
            "system",
            format!("{}/{}/{}/{}", d.hostname, d.timezone, d.locale, d.keymap),
        ),
        (
            "partitions",
            format!(
                "esp {}  slot_a {}  slot_b {}  var {}  home {}",
                d.esp, d.slot_a, d.slot_b, d.var, d.home
            ),
        ),
        (
            "filesystems",
            format!("slot erofs  var {}  home {}", fs_name(d.fs_var), fs_name(d.fs_home)),
        ),
        (
            "encryption",
            format!("var {}  home {}", enc_name(d.enc_var), enc_name(d.enc_home)),
        ),
        ("users", users.join(", ")),
        ("ssh", format!("{} authorized key(s)", d.ssh_keys.len())),
        ("services", format!("{} unit(s)", d.services.len())),
    ];
    for (label, value) in rows {
        let value: String = value.chars().take(w.saturating_sub(LABEL_W + 5)).collect();
        out.push(Line::from(vec![
            Span::styled(format!("  {:<LABEL_W$}", label), bold()),
            Span::raw(value),
        ]));
    }
    if ui.errors.is_empty() {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            "  validation: no errors (strict parse + engine plan)",
            green(),
        )));
    } else {
        out.push(Line::default());
        for e in &ui.errors {
            out.push(Line::from(Span::styled(format!("  ! {e}"), red())));
        }
    }
}

fn plan_lines(ui: &Ui, w: usize, out: &mut Vec<Line<'static>>) {
    out.push(Line::default());
    match &ui.plan {
        Ok(report) => {
            out.push(Line::from(Span::styled(
                format!(" plan ok - config: {}", ui.config_out.display()),
                green(),
            )));
            out.push(Line::default());
            for l in report.lines() {
                let l: String = l.chars().take(w.saturating_sub(4)).collect();
                out.push(Line::from(Span::raw(format!("  {l}"))));
            }
            if ui.dry_run {
                out.push(Line::default());
                out.push(Line::from(Span::styled(
                    " dry run: the disk is not touched",
                    dim(),
                )));
            }
        }
        Err(e) => {
            out.push(Line::from(Span::styled(format!(" plan failed: {e}"), red())));
        }
    }
}

fn confirm_lines(
    ui: &Ui,
    w: usize,
    out: &mut Vec<Line<'static>>,
    cursor: &mut Option<(u16, u16)>,
) {
    out.push(Line::default());
    let target: String = ui
        .draft
        .target_disk
        .chars()
        .take(w.saturating_sub(42))
        .collect();
    out.push(Line::from(Span::styled(
        format!(" WARNING: the install destroys all existing content on {target}."),
        red().add_modifier(Modifier::BOLD),
    )));
    out.push(Line::from(Span::styled(
        format!(" The config file is written at {} and the", ui.config_out.display()),
        red(),
    )));
    out.push(Line::from(Span::styled(
        " engine runs on that file - the same path as engine mode.",
        red(),
    )));
    out.push(Line::default());
    out.push(Line::from(vec![Span::styled("  > ", bold()), Span::raw(ui.confirm.clone())]));
    *cursor = Some(((4 + ui.confirm.len()) as u16, 5));
}

fn done_lines(ui: &Ui, out: &mut Vec<Line<'static>>) {
    out.push(Line::default());
    let title = if ui.dry_run {
        " dry run complete - no disk was touched"
    } else {
        " install complete"
    };
    out.push(Line::from(Span::styled(
        title,
        green().add_modifier(Modifier::BOLD),
    )));
    out.push(Line::from(format!("  config: {}", ui.config_out.display())));
    out.push(Line::from(format!(
        "  log:    {} (copied to the target's /var/lib/ingot/install.log)",
        ui.work.join("install.log").display()
    )));
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
