//! The wizard's ratatui front-end: one screen per 11.4 category,
//! then the review, the plan, and the confirmation.
//!
//! The UI is a thin layer over `wizard::Draft`: every diagnostic
//! shown on a screen comes from the engine's strict parser (via
//! `Draft::config`), the plan shown before the confirmation is the
//! engine's own plan computed from the written config file, and the
//! install is the engine running on that file - the same execution
//! path as engine mode (spec 11.3).

use crate::config;
use crate::engine;
use crate::wizard::{Draft, Step};
use crossterm::event::{self, Event, KeyEventKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod ui_key;
mod ui_draw;

/// What the wizard mode ended with (drives the process exit code).
#[derive(Clone)]
pub enum Outcome {
    /// Install complete, or the dry-run plan was reported.
    Done,
    /// Aborted by the user; the target disk was not touched.
    Aborted,
    /// The engine (or the config write) failed with this error.
    Failed(String),
}

const POLL: Duration = Duration::from_millis(50);

/// The wizard screens.
enum Screen {
    /// One of the ten Steps (the review is the last Step).
    Step,
    /// The engine's plan report (or its error) from the config file.
    Plan,
    /// The 11.6.2 confirmation: type `yes` to install.
    Confirm,
    /// Terminal screen before exiting.
    Done,
}


/// The wizard's mutable state.
struct Ui {
    draft: Draft,
    step: Step,
    screen: Screen,
    /// Selection within the current screen (field row or list row).
    sel: usize,
    /// Users screen: which sub-field of the selected user (0 name, 1 shell).
    sub: usize,
    /// Confirmation-screen input so far.
    confirm: String,
    /// A modal y/n prompt (abort confirmation), if any.
    prompt: Option<String>,
    /// Transient hint (cleared on the next key).
    flash: Option<String>,
    /// All validation diagnostics (the review gate).
    errors: Vec<String>,
    /// The engine's plan report, or the plan's error.
    plan: Result<String, String>,
    config_out: PathBuf,
    /// True once the wizard wrote the config file itself: later
    /// review passes re-write it instead of refusing an existing
    /// file (a file the user pointed --config at is never touched).
    config_written: bool,
    work: PathBuf,
    dry_run: bool,
    result: Option<Outcome>,
}

impl Ui {
    fn new(config_out: PathBuf, work: PathBuf, dry_run: bool) -> Self {
        Self {
            draft: Draft::new(),
            step: Step::Target,
            screen: Screen::Step,
            sel: 0,
            sub: 0,
            confirm: String::new(),
            prompt: None,
            flash: None,
            errors: Vec::new(),
            plan: Err(String::new()),
            config_out,
            config_written: false,
            work,
            dry_run,
            result: None,
        }
    }

    fn rows(&self) -> usize {
        match self.step {
            Step::Target => 1,
            Step::Source => 2,
            Step::System => 4,
            Step::Partitions => 5,
            Step::Filesystems => 3,
            Step::Encryption => 2,
            Step::Users => self.draft.users.len(),
            Step::Ssh => self.draft.ssh_keys.len(),
            Step::Services => self.draft.services.len(),
            Step::Review => 0,
        }
    }

    /// The engine's diagnostics filtered to the current step's
    /// category (the review sees all of them).
    fn step_errors(&self) -> Vec<String> {
        let Ok(cfg) = self.draft.config() else {
            let all = self.draft.config().unwrap_err();
            return filter_errors(all, self.step);
        };
        let _ = cfg;
        Vec::new()
    }

    /// The review gate: the strict-parse diagnostics plus the
    /// engine's own read-only plan diagnostics (the engine rejects
    /// what it cannot do - e.g. luks2 in v1 - and the user sees
    /// that here, not at the plan).
    fn enter_review(&mut self) {
        self.errors = match self.draft.config() {
            Err(e) => e,
            Ok(cfg) => match engine::plan(cfg) {
                Ok(_) => Vec::new(),
                Err(e) => vec![e],
            },
        };
    }

    /// Review gate -> write the config file, re-read and re-parse it,
    /// and compute the engine's plan from it.
    fn advance_to_plan(&mut self) {
        let cfg = match self.draft.config() {
            Ok(c) => c,
            Err(e) => {
                self.errors = e;
                return;
            }
        };
        let text = config::render(&cfg);
        if self.config_out.exists() && !self.config_written {
            self.plan = Err(format!(
                "config {} already exists; remove it or choose another --config path",
                self.config_out.display()
            ));
            self.screen = Screen::Plan;
            return;
        }
        match std::fs::write(&self.config_out, &text) {
            Ok(()) => {
                self.config_written = true;
                self.plan_from_file();
            }
            Err(e) => {
                self.plan = Err(format!(
                    "cannot write config {}: {e}",
                    self.config_out.display()
                ));
                self.screen = Screen::Plan;
            }
        }
    }

    /// The single execution path: plan the config file the wizard
    /// just wrote (engine mode plans the same file).
    fn plan_from_file(&mut self) {
        self.plan = std::fs::read_to_string(&self.config_out)
            .map_err(|e| format!("cannot re-read config {}: {e}", self.config_out.display()))
            .and_then(|t| {
                config::parse(&t)
                    .map_err(|errs| format!("config {} did not survive the strict parse: {}", self.config_out.display(), errs.join("; ")))
            })
            .and_then(|cfg| {
                engine::plan(cfg)
                    .map(|p| engine::render_report(&p))
                    .map_err(|e| format!("plan failed: {e}"))
            });
        self.screen = Screen::Plan;
    }

    /// Confirmation -> run the engine on the written config file.
    fn confirm_run(&mut self) {
        let text = match std::fs::read_to_string(&self.config_out) {
            Ok(t) => t,
            Err(e) => {
                self.result = Some(Outcome::Failed(format!(
                    "cannot re-read config {}: {e}",
                    self.config_out.display()
                )));
                return;
            }
        };
        let cfg = match config::parse(&text) {
            Ok(c) => c,
            Err(errs) => {
                self.result = Some(Outcome::Failed(errs.join("; ")));
                return;
            }
        };
        // The screen cannot redraw while the engine runs: note it.
        self.flash = Some(format!(
            "installing... (the engine is running; log: {})",
            self.work.join("install.log").display()
        ));
        match engine::run(cfg, &self.work) {
            Ok(()) => {
                self.flash = None;
                self.screen = Screen::Done;
            }
            Err(e) => {
                self.flash = None;
                self.result = Some(Outcome::Failed(e));
            }
        }
    }

}

/// Diagnostics belonging to one step's category (all for the review).
fn filter_errors(errs: Vec<String>, step: Step) -> Vec<String> {
    let prefix = match step {
        Step::Target => "[target]",
        Step::Source => "[source]",
        Step::System => "[system]",
        Step::Partitions => "[partitions]",
        Step::Filesystems => "[filesystems]",
        Step::Encryption => "[encryption]",
        Step::Users => "[[users]]",
        Step::Ssh => "[ssh]",
        Step::Services => "[services]",
        Step::Review => "",
    };
    if prefix.is_empty() {
        errs
    } else {
        errs.into_iter().filter(|e| e.starts_with(prefix)).collect()
    }
}

/// Runs the interactive wizard: collect the draft (all 11.4
/// categories), show the engine's plan from the written config,
/// require explicit confirmation (11.6.2), then run the engine on
/// that config. `dry_run` stops after the plan (exit 0, no install).
pub fn run(config_out: &Path, work: &Path, dry_run: bool) -> Outcome {
    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            return Outcome::Failed(format!(
                "cannot initialize the terminal for the wizard: {e}"
            ));
        }
    };
    let mut ui = Ui::new(config_out.to_path_buf(), work.to_path_buf(), dry_run);
    let outcome = loop {
        if let Some(r) = ui.result.clone() {
            break r;
        }
        if let Err(e) = ui_draw::draw(&mut terminal, &mut ui) {
            ratatui::restore();
            return Outcome::Failed(format!("terminal error: {e}"));
        }
        if !event::poll(POLL).unwrap_or(false) {
            continue;
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => ui.key(k),
            Ok(_) => {}
            Err(e) => {
                ratatui::restore();
                return Outcome::Failed(format!("terminal error: {e}"));
            }
        }
    };
    ratatui::restore();
    match &outcome {
        Outcome::Done => {
            println!("{}", done_text(&ui));
        }
        Outcome::Aborted => {
            eprintln!("wizard aborted; the target disk was not touched");
        }
        Outcome::Failed(_) => {} // main prints the error
    }
    outcome
}

/// The plain-text summary printed after the TUI is restored.
fn done_text(ui: &Ui) -> String {
    let mut s = String::new();
    if ui.dry_run {
        s.push_str("dry run complete; the disk was not touched\n");
        if let Ok(report) = &ui.plan {
            s.push_str(report);
        }
    } else {
        s.push_str("install complete\n");
    }
    s.push_str(&format!("  config: {}\n", ui.config_out.display()));
    s.push_str(&format!(
        "  log: {} (copied to the target's /var/lib/ingot/install.log)\n",
        ui.work.join("install.log").display()
    ));
    s
}

