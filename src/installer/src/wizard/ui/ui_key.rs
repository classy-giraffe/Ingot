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
            KeyCode::Up if self.step == Step::Target => self.cycle_disk(false),
            KeyCode::Down if self.step == Step::Target => self.cycle_disk(true),
            KeyCode::Up => {
                self.sel = self.rows().saturating_sub(self.sel + 1);
            }
            KeyCode::Down => {
                if self.sel + 1 < self.rows() {
                    self.sel += 1;
                }
            }
            KeyCode::Left if self.step == Step::Target => self.cycle_disk(false),
            KeyCode::Right if self.step == Step::Target => self.cycle_disk(true),
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
    fn cycle_disk(&mut self, forward: bool) {
        let disks = crate::target::probe_disks();
        if disks.is_empty() {
            return;
        }
        let cur = &self.draft.target_disk;
        let pos = disks.iter().position(|d| d.path.to_string_lossy() == *cur);
        let next = match pos {
            Some(i) if forward => (i + 1) % disks.len(),
            Some(i) => (i + disks.len() - 1) % disks.len(),
            None => 0,
        };
        self.draft.target_disk = disks[next].path.to_string_lossy().to_string();
    }

    /// The editable string under the cursor (the selected row's
    /// field, or the selected scalar), or None where the position is
    /// not editable: the fixed slot filesystem, the left/right
    /// cycled encryption choice, or the review screen.
    fn field(&mut self) -> Option<&mut String> {
        match self.step {
            Step::Target => Some(&mut self.draft.target_disk),
            Step::Source => match self.sel {
                0 => Some(&mut self.draft.source_base),
                1 => Some(&mut self.draft.version),
                _ => None,
            },
            Step::System => match self.sel {
                0 => Some(&mut self.draft.hostname),
                1 => Some(&mut self.draft.timezone),
                2 => Some(&mut self.draft.locale),
                _ => Some(&mut self.draft.keymap),
            },
            Step::Partitions => match self.sel {
                0 => Some(&mut self.draft.esp),
                1 => Some(&mut self.draft.slot_a),
                2 => Some(&mut self.draft.slot_b),
                3 => Some(&mut self.draft.var),
                _ => Some(&mut self.draft.home),
            },
            Step::Users => {
                let u = self.draft.users.get_mut(self.sel)?;
                if self.sub == 0 {
                    Some(&mut u.name)
                } else {
                    Some(&mut u.shell)
                }
            }
            Step::Ssh => self.draft.ssh_keys.get_mut(self.sel),
            Step::Services => self.draft.services.get_mut(self.sel),
            _ => None,
        }
    }

    /// True while the cursor is on the source-base row: the version
    /// is re-derived from the base after every edit there.
    fn editing_base(&self) -> bool {
        self.step == Step::Source && self.sel == 0
    }

    fn append(&mut self, c: char) {
        if let Some(f) = self.field() {
            f.push(c);
        }
        if self.editing_base() {
            self.draft.refresh_version();
        }
    }

    fn backspace(&mut self) {
        if let Some(f) = self.field() {
            f.pop();
        }
        if self.editing_base() {
            self.draft.refresh_version();
        }
    }

    fn add_row(&mut self) {
        // Insert after the selected row; on an empty list (or past
        // the end) that is the empty list itself - clamped so the
        // cursor lands on the new row.
        let at = |len: usize, sel: usize| (sel + 1).min(len);
        match self.step {
            Step::Users => {
                let i = at(self.draft.users.len(), self.sel);
                self.draft.users.insert(
                    i,
                    wizard::DraftUser {
                        name: String::new(),
                        shell: String::new(),
                    },
                );
                self.sel = i;
            }
            Step::Ssh => {
                let i = at(self.draft.ssh_keys.len(), self.sel);
                self.draft.ssh_keys.insert(i, String::new());
                self.sel = i;
            }
            Step::Services => {
                let i = at(self.draft.services.len(), self.sel);
                self.draft.services.insert(i, String::new());
                self.sel = i;
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

#[cfg(test)]
mod tests {
    use super::Ui;
    use crate::wizard::Step;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn ui() -> Ui {
        Ui::new(PathBuf::from("/tmp/t.toml"), PathBuf::from("/tmp/w"), false)
    }
    fn press(ui: &mut Ui, code: KeyCode) {
        ui.key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    /// Walk the steps forward to the given screen.
    fn to(ui: &mut Ui, step: Step) {
        while ui.step != step {
            press(ui, KeyCode::Enter);
        }
    }

    // The Ssh and Services screens start empty: Insert must add a row
    // without panicking (Vec::insert past the end) and leave the
    // cursor on the new row.
    #[test]
    fn insert_on_empty_lists() {
        let mut ui = ui();
        to(&mut ui, Step::Ssh);
        assert!(ui.draft.ssh_keys.is_empty());
        press(&mut ui, KeyCode::Insert);
        assert_eq!(ui.draft.ssh_keys, [String::new()]);
        assert_eq!(ui.sel, 0);

        to(&mut ui, Step::Services);
        assert!(ui.draft.services.is_empty());
        press(&mut ui, KeyCode::Insert);
        assert_eq!(ui.draft.services, [String::new()]);
        assert_eq!(ui.sel, 0);
    }

    // Users starts with one user: deleting it all and adding again
    // must not panic either.
    #[test]
    fn insert_after_deleting_all_users() {
        let mut ui = ui();
        to(&mut ui, Step::Users);
        press(&mut ui, KeyCode::Delete);
        assert!(ui.draft.users.is_empty());
        press(&mut ui, KeyCode::Insert);
        assert_eq!(ui.draft.users.len(), 1);
        assert_eq!(ui.sel, 0);
    }

    // Inserting mid-list lands the cursor on the new row.
    #[test]
    fn insert_after_selected_row() {
        let mut ui = ui();
        to(&mut ui, Step::Users);
        press(&mut ui, KeyCode::Insert); // after the (single) default user
        assert_eq!(ui.draft.users.len(), 2);
        assert_eq!(ui.sel, 1);
    }
}
