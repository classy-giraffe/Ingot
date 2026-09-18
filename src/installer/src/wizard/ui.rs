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
use crate::wizard::{self, Draft, Step};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

    fn enter_review(&mut self) {
        self.errors = match self.draft.config() {
            Ok(_) => Vec::new(),
            Err(e) => e,
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
        if self.config_out.exists() {
            self.plan = Err(format!(
                "config {} already exists; remove it or choose another --config path",
                self.config_out.display()
            ));
            self.screen = Screen::Plan;
            return;
        }
        match std::fs::write(&self.config_out, &text) {
            Ok(()) => self.plan_from_file(),
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

    fn key(&mut self, k: KeyEvent) {
        self.flash = None;
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt = Some("Abort the install? The target disk is untouched. (y/n)".into());
                return;
            }
            _ => {}
        }
        if self.prompt.is_some() {
            match k.code {
                KeyCode::Char('y') => self.result = Some(Outcome::Aborted),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => self.prompt = None,
                _ => {}
            }
            return;
        }
        match self.screen {
            Screen::Step => self.step_key(k),
            Screen::Plan => self.plan_key(k),
            Screen::Confirm => self.confirm_key(k),
            Screen::Done => {
                if k.code == KeyCode::Enter {
                    self.result = Some(Outcome::Done);
                }
            }
        }
    }

    fn step_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                if let Some(p) = self.step.prev() {
                    self.step = p;
                    self.sel = 0;
                    self.sub = 0;
                    if self.step == Step::Review {
                        self.enter_review();
                    }
                } else {
                    self.prompt = Some("Abort the install? The target disk is untouched. (y/n)".into());
                }
            }
            KeyCode::Enter | KeyCode::Tab => {
                if self.step == Step::Review {
                    if self.errors.is_empty() {
                        self.advance_to_plan();
                    } else {
                        self.flash = Some("fix the validation errors first (esc to edit)".into());
                    }
                } else if let Some(n) = self.step.next() {
                    self.step = n;
                    self.sel = 0;
                    self.sub = 0;
                    if n == Step::Review {
                        self.enter_review();
                    }
                }
            }
            KeyCode::BackTab => {
                if let Some(p) = self.step.prev() {
                    self.step = p;
                    self.sel = 0;
                    self.sub = 0;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.sel = self.rows().saturating_sub(self.sel + 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.sel + 1 < self.rows() {
                    self.sel += 1;
                }
            }
            KeyCode::Left if self.step == Step::Users => self.sub = 0,
            KeyCode::Right if self.step == Step::Users => self.sub = 1,
            KeyCode::Left | KeyCode::Right => self.cycle(k.code),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char(c) => match self.step {
                Step::Users | Step::Ssh | Step::Services => match c {
                    'a' => self.add_row(),
                    'x' => self.del_row(),
                    other if !other.is_control() => self.append(other),
                    _ => {}
                },
                _ => {
                    if !c.is_control() {
                        self.append(c);
                    }
                }
            },
            _ => {}
        }
    }

    fn cycle(&mut self, code: KeyCode) {
        let dir = if code == KeyCode::Left { -1 } else { 1 };
        match self.step {
            Step::Filesystems => match self.sel {
                0 => {} // the slot filesystem is fixed in v1
                1 => self.draft.fs_var = if dir < 0 {
                    crate::config::StateFs::Ext4
                } else {
                    crate::config::StateFs::Btrfs
                },
                2 => self.draft.fs_home = if dir < 0 {
                    crate::config::StateFs::Ext4
                } else {
                    crate::config::StateFs::Btrfs
                },
                _ => {}
            },
            Step::Encryption => {
                let (cur, next) = if dir < 0 {
                    (
                        crate::config::Encryption::None,
                        crate::config::Encryption::Luks2,
                    )
                } else {
                    (
                        crate::config::Encryption::Luks2,
                        crate::config::Encryption::None,
                    )
                };
                let _ = cur;
                let e = match self.sel {
                    0 => &mut self.draft.enc_var,
                    _ => &mut self.draft.enc_home,
                };
                *e = next;
            }
            _ => {}
        }
    }

    fn append(&mut self, c: char) {
        match self.step {
            Step::Target => self.draft.target_disk.push(c),
            Step::Source => match self.sel {
                0 => {
                    self.draft.source_base.push(c);
                    self.draft.refresh_version();
                }
                1 => self.draft.version.push(c),
                _ => {}
            },
            Step::System => match self.sel {
                0 => self.draft.hostname.push(c),
                1 => self.draft.timezone.push(c),
                2 => self.draft.locale.push(c),
                _ => self.draft.keymap.push(c),
            },
            Step::Partitions => match self.sel {
                0 => self.draft.esp.push(c),
                1 => self.draft.slot_a.push(c),
                2 => self.draft.slot_b.push(c),
                3 => self.draft.var.push(c),
                _ => self.draft.home.push(c),
            },
            Step::Users => {
                if let Some(u) = self.draft.users.get_mut(self.sel) {
                    if self.sub == 0 {
                        u.name.push(c)
                    } else {
                        u.shell.push(c)
                    }
                }
            }
            Step::Ssh => {
                if let Some(k) = self.draft.ssh_keys.get_mut(self.sel) {
                    k.push(c)
                }
            }
            Step::Services => {
                if let Some(u) = self.draft.services.get_mut(self.sel) {
                    u.push(c)
                }
            }
            _ => {}
        }
    }

    fn backspace(&mut self) {
        match self.step {
            Step::Target => {
                self.draft.target_disk.pop();
            }
            Step::Source => match self.sel {
                0 => {
                    self.draft.source_base.pop();
                    self.draft.refresh_version();
                }
                1 => {
                    self.draft.version.pop();
                }
                _ => {}
            },
            Step::System => match self.sel {
                0 => {
                    self.draft.hostname.pop();
                }
                1 => {
                    self.draft.timezone.pop();
                }
                2 => {
                    self.draft.locale.pop();
                }
                _ => {
                    self.draft.keymap.pop();
                }
            },
            Step::Partitions => match self.sel {
                0 => {
                    self.draft.esp.pop();
                }
                1 => {
                    self.draft.slot_a.pop();
                }
                2 => {
                    self.draft.slot_b.pop();
                }
                3 => {
                    self.draft.var.pop();
                }
                _ => {
                    self.draft.home.pop();
                }
            },
            Step::Users => {
                if let Some(u) = self.draft.users.get_mut(self.sel) {
                    if self.sub == 0 {
                        u.name.pop();
                    } else {
                        u.shell.pop();
                    }
                }
            }
            Step::Ssh => {
                if let Some(k) = self.draft.ssh_keys.get_mut(self.sel) {
                    k.pop();
                }
            }
            Step::Services => {
                if let Some(u) = self.draft.services.get_mut(self.sel) {
                    u.pop();
                }
            }
            _ => {}
        }
    }

    fn add_row(&mut self) {
        match self.step {
            Step::Users => {
                self.draft.users.insert(
                    self.sel + 1,
                    wizard::DraftUser {
                        name: String::new(),
                        shell: String::new(),
                    },
                );
                self.sel += 1;
            }
            Step::Ssh => {
                self.draft.ssh_keys.insert(self.sel + 1, String::new());
                self.sel += 1;
            }
            Step::Services => {
                self.draft.services.insert(self.sel + 1, String::new());
                self.sel += 1;
            }
            _ => {}
        }
    }

    fn del_row(&mut self) {
        match self.step {
            Step::Users => {
                if self.sel < self.draft.users.len() {
                    self.draft.users.remove(self.sel);
                    if self.sel > 0 {
                        self.sel -= 1;
                    }
                }
            }
            Step::Ssh => {
                if self.sel < self.draft.ssh_keys.len() {
                    self.draft.ssh_keys.remove(self.sel);
                    if self.sel > 0 {
                        self.sel -= 1;
                    }
                }
            }
            Step::Services => {
                if self.sel < self.draft.services.len() {
                    self.draft.services.remove(self.sel);
                    if self.sel > 0 {
                        self.sel -= 1;
                    }
                }
            }
            _ => {}
        }
    }

    fn plan_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => {
                if self.plan.is_err() {
                    self.flash = Some("the plan failed; esc to go back and edit".into());
                    return;
                }
                if self.dry_run {
                    self.screen = Screen::Done;
                } else {
                    self.confirm = String::new();
                    self.screen = Screen::Confirm;
                }
            }
            KeyCode::Esc => {
                self.step = Step::Review;
                self.screen = Screen::Step;
                self.enter_review();
            }
            _ => {}
        }
    }

    fn confirm_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => {
                if self.confirm == "yes" {
                    self.confirm_run();
                } else {
                    self.flash = Some("type 'yes' and press enter to install".into());
                }
            }
            KeyCode::Esc => {
                self.prompt = Some(
                    "Abort the install? The target disk is untouched. (y/n)".into(),
                );
            }
            KeyCode::Backspace => {
                self.confirm.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.confirm.push(c);
            }
            _ => {}
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

