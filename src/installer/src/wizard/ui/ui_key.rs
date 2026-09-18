//! Key-input handling for the wizard: the per-screen dispatch,
//! navigation, field editing, row add/remove, and the plan and
//! confirmation key handlers.
//!
//! Row navigation is arrow-keys only and row actions are the
//! Insert/Delete keys: printable characters must never be
//! intercepted, because they are typed into fields (paths, hostnames,
//! SSH keys).

use super::{Outcome, Screen, Step, Ui};
use crate::wizard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Ui {
    pub(super) fn key(&mut self, k: KeyEvent) {
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
            KeyCode::Up => {
                self.sel = self.rows().saturating_sub(self.sel + 1);
            }
            KeyCode::Down => {
                if self.sel + 1 < self.rows() {
                    self.sel += 1;
                }
            }
            KeyCode::Left if self.step == Step::Users => self.sub = 0,
            KeyCode::Right if self.step == Step::Users => self.sub = 1,
            KeyCode::Left | KeyCode::Right => self.cycle(),
            KeyCode::Insert if matches!(
                self.step,
                Step::Users | Step::Ssh | Step::Services
            ) => {
                self.add_row()
            }
            KeyCode::Delete if matches!(
                self.step,
                Step::Users | Step::Ssh | Step::Services
            ) => {
                self.del_row()
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char(c) if !c.is_control() => self.append(c),
            _ => {}
        }
    }

    fn cycle(&mut self) {
        // left and right both cycle the two v1 choices
        match self.step {
            Step::Filesystems => match self.sel {
                0 => {} // the slot filesystem is fixed in v1
                1 => {
                    self.draft.fs_var = match self.draft.fs_var {
                        crate::config::StateFs::Ext4 => crate::config::StateFs::Btrfs,
                        crate::config::StateFs::Btrfs => crate::config::StateFs::Ext4,
                    }
                }
                2 => {
                    self.draft.fs_home = match self.draft.fs_home {
                        crate::config::StateFs::Ext4 => crate::config::StateFs::Btrfs,
                        crate::config::StateFs::Btrfs => crate::config::StateFs::Ext4,
                    }
                }
                _ => {}
            },
            Step::Encryption => {
                let e = match self.sel {
                    0 => &mut self.draft.enc_var,
                    _ => &mut self.draft.enc_home,
                };
                *e = match *e {
                    crate::config::Encryption::None => crate::config::Encryption::Luks2,
                    crate::config::Encryption::Luks2 => crate::config::Encryption::None,
                };
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
