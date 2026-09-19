//! Main drawing coordinator for the Ingot Installer TUI.
//!
//! Renders a modern, ncurses-inspired layout:
//! - Full-width top header banner with OS identity
//! - Step breadcrumb navigation bar
//! - Rounded border card for the active screen
//! - Status footer with keybinding assistance

use super::{Screen, Step, Ui};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::DefaultTerminal;
use std::io;

/// Draws one frame of the installer wizard.
pub fn draw(terminal: &mut DefaultTerminal, ui: &mut Ui) -> io::Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(1), // Top header banner
            Constraint::Length(1), // Step breadcrumb ribbon
            Constraint::Min(5),    // Main card content
            Constraint::Length(1), // Footer key hints
        ])
        .split(area);

        let (header_area, ribbon_area, content_area, footer_area) =
            (chunks[0], chunks[1], chunks[2], chunks[3]);

        // 1. Top Header Banner
        let header = Paragraph::new(render_header(ui, header_area.width as usize));
        f.render_widget(header, header_area);

        // 2. Step Breadcrumb Ribbon
        let ribbon = Paragraph::new(render_ribbon(ui, ribbon_area.width as usize));
        f.render_widget(ribbon, ribbon_area);

        // 3. Main Card Content
        let (lines, cursor) = render_content(ui, content_area.width as usize);
        let title_line = Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(screen_title(ui), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::default()),
        ]);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title_line);
        let para = Paragraph::new(lines).block(block);
        f.render_widget(para, content_area);

        // 4. Footer Key Hints
        let hint = ui.flash.clone().unwrap_or_else(|| footer_hint(ui).to_string());
        let footer_line = Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(hint, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(footer_line), footer_area);

        // Position terminal cursor if an editable field is active
        if let Some((x, y)) = cursor {
            f.set_cursor_position((
                (content_area.x + x).min(content_area.right().saturating_sub(1)),
                (content_area.y + y).min(content_area.bottom().saturating_sub(1)),
            ));
        }
    })
    .map(|_| ())
}

fn render_header(ui: &Ui, w: usize) -> Line<'static> {
    let title = "  Ingot OS  0.1.0  Installation Wizard";
    let step_info = match ui.screen {
        Screen::Step => format!("Step {} of 10  ", ui.step.index() + 1),
        Screen::Plan => "System Plan  ".to_string(),
        Screen::Confirm => "Confirmation  ".to_string(),
        Screen::Done => "Completed  ".to_string(),
    };
    let pad = w.saturating_sub(title.len() + step_info.len());
    let spans = vec![
        Span::styled(title, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" ".repeat(pad), Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled(step_info, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ];
    Line::from(spans)
}

fn render_ribbon(ui: &Ui, _w: usize) -> Line<'static> {
    if ui.screen != Screen::Step {
        let msg = match ui.screen {
            Screen::Plan => " Reviewing partition plan computed from configuration",
            Screen::Confirm => " Awaiting confirmation before writing to disk",
            Screen::Done => " Installation successfully completed",
            Screen::Step => "",
        };
        return Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray)));
    }

    let steps = [
        "1.Disk", "2.Source", "3.System", "4.Layout", "5.FS", "6.Security", "7.Users", "8.SSH", "9.Services", "10.Review",
    ];

    let mut spans = vec![Span::raw(" ")];
    let cur = ui.step.index();
    for (i, label) in steps.iter().enumerate() {
        if i < cur {
            spans.push(Span::styled(*label, Style::default().fg(Color::Green)));
        } else if i == cur {
            spans.push(Span::styled(format!("[{label}]"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(*label, Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

fn screen_title(ui: &Ui) -> String {
    match ui.screen {
        Screen::Step => format!("{} ({} of 10)", ui.step.title(), ui.step.index() + 1),
        Screen::Plan => "Computed Partition Plan".into(),
        Screen::Confirm => "Confirm Destructive Installation".into(),
        Screen::Done => "Installation Complete".into(),
    }
}

fn render_content(ui: &Ui, w: usize) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cursor: Option<(u16, u16)> = None;

    match ui.screen {
        Screen::Step => {
            if ui.step == Step::Review {
                super::ui_review::review_lines(ui, w, &mut out);
            } else {
                super::ui_steps::step_lines(ui, w, &mut out, &mut cursor);
                let errs = ui.step_errors();
                if !errs.is_empty() {
                    out.push(Line::default());
                    for e in &errs {
                        out.push(Line::from(Span::styled(format!("  ! {e}"), Style::default().fg(Color::Red))));
                    }
                }
            }
        }
        Screen::Plan => super::ui_review::plan_lines(ui, w, &mut out),
        Screen::Confirm => super::ui_review::confirm_lines(ui, w, &mut out, &mut cursor),
        Screen::Done => super::ui_review::done_lines(ui, &mut out),
    }

    if let Some(p) = &ui.prompt {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            format!("  {p}"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        cursor = None;
    }

    (out, cursor)
}

fn footer_hint(ui: &Ui) -> &'static str {
    match ui.screen {
        Screen::Step => match ui.step {
            Step::Target => " [Left/Right] Cycle Disks   [Enter] Next Step   [Esc] Abort",
            Step::Filesystems | Step::Encryption => " [Up/Down] Move Field   [Left/Right] Toggle Option   [Enter] Next   [Esc] Back",
            Step::Users => " [Up/Down] Select User   [Left/Right] Switch Field   [Insert] Add   [Delete] Remove   [Enter] Next   [Esc] Back",
            Step::Ssh | Step::Services => " [Up/Down] Select Item   [Insert] Add Entry   [Delete] Remove   [Enter] Next   [Esc] Back",
            Step::Review => " [Enter] Write Config & Show Plan   [Esc] Back to Edit   [Ctrl+C] Abort",
            _ => " [Up/Down] Move Field   [Enter] Next Step   [Esc] Previous Step   [Ctrl+C] Abort",
        },
        Screen::Plan => {
            if ui.dry_run {
                " [Enter] Complete Dry Run (no disk changes)   [Esc] Back to Review"
            } else if ui.plan.is_err() {
                " Plan generation failed   [Esc] Back to Edit Configuration"
            } else {
                " [Enter] Proceed to Confirmation   [Esc] Back to Review"
            }
        }
        Screen::Confirm => " Type 'yes' and press [Enter] to install   [Esc] Abort and return to Plan",
        Screen::Done => " [Enter] Exit to Terminal",
    }
}
