//! The interactive wizard's state (spec 11.3): the 11.4 input
//! categories as editable draft fields, one screen per category,
//! then the review.
//!
//! The draft is deliberately raw: field values are still-unparsed
//! strings so the operator can type them. `Draft::config` is the
//! single validation gate - it parses the fields, renders the
//! config to TOML, and re-parses the result with the engine's
//! strict parser, so the wizard reports exactly the diagnostics the
//! engine would. The wizard never installs: it writes the TOML
//! config the engine already runs.

use crate::config::{self, Encryption, SlotFs, StateFs, User};
use crate::size::ByteSize;
use crate::version::Version;
use std::path::Path;

/// One user entry while being edited. `shell` is the raw field:
/// empty means the default shell (it is not rendered, so the engine
/// applies `DEFAULT_SHELL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftUser {
    pub name: String,
    pub shell: String,
}

/// The wizard's in-progress install config: all eleven spec-11.4
/// categories as editable fields.
#[derive(Debug, Clone)]
pub struct Draft {
    // 11.4.1 target disk
    pub target_disk: String,
    // 11.4.2 OS image source
    pub source_base: String,
    pub version: String,
    // 11.4.3-5 system identity
    pub hostname: String,
    pub timezone: String,
    pub locale: String,
    pub keymap: String,
    // 11.4.6 partition sizes (raw size strings)
    pub esp: String,
    pub slot_a: String,
    pub slot_b: String,
    pub var: String,
    pub home: String,
    // 11.4.7 filesystem choices
    pub fs_slot: SlotFs,
    pub fs_var: StateFs,
    pub fs_home: StateFs,
    // 11.4.8 encryption choices
    pub enc_var: Encryption,
    pub enc_home: Encryption,
    // 11.4.9 initial users
    pub users: Vec<DraftUser>,
    // 11.4.10 SSH authorized keys
    pub ssh_keys: Vec<String>,
    // 11.4.11 service enablement policy
    pub services: Vec<String>,
}

/// The wizard's screens: the 11.4 categories in spec order, then
/// the review. The review is the last screen; the plan and the
/// confirmation follow it (see `wizard::ui`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Target,
    Source,
    System,
    Partitions,
    Filesystems,
    Encryption,
    Users,
    Ssh,
    Services,
    Review,
}

impl Step {
    pub const ALL: [Step; 10] = [
        Step::Target,
        Step::Source,
        Step::System,
        Step::Partitions,
        Step::Filesystems,
        Step::Encryption,
        Step::Users,
        Step::Ssh,
        Step::Services,
        Step::Review,
    ];

    /// The screen's human title (the category, not its number).
    pub fn title(self) -> &'static str {
        match self {
            Step::Target => "target disk (11.4.1)",
            Step::Source => "OS image source (11.4.2)",
            Step::System => "system identity (11.4.3-5)",
            Step::Partitions => "partition sizes (11.4.6)",
            Step::Filesystems => "filesystem choices (11.4.7)",
            Step::Encryption => "encryption choices (11.4.8)",
            Step::Users => "initial users (11.4.9)",
            Step::Ssh => "SSH authorized keys (11.4.10)",
            Step::Services => "service enablement (11.4.11)",
            Step::Review => "review",
        }
    }

    pub fn index(self) -> usize {
        (self as u8) as usize
    }

    pub fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }

    pub fn prev(self) -> Option<Step> {
        if self.index() == 0 {
            None
        } else {
            Some(Step::ALL[self.index() - 1])
        }
    }
}

impl Draft {
    /// Sensible defaults (workstation profile): the operator must
    /// still fill the target disk; the source defaults to `dist` in
    /// the current directory with the newest available version.
    pub fn new() -> Self {
        let source_base = "dist".to_string();
        let version = source_version(&source_base);
        Self {
            target_disk: String::new(),
            source_base,
            version,
            hostname: "ingot".into(),
            timezone: "UTC".into(),
            locale: "C.UTF-8".into(),
            keymap: "us".into(),
            esp: "1G".into(),
            slot_a: "8G".into(),
            slot_b: "8G".into(),
            var: "4G".into(),
            home: "8G".into(),
            fs_slot: SlotFs::Eros,
            fs_var: StateFs::Btrfs,
            fs_home: StateFs::Btrfs,
            enc_var: Encryption::None,
            enc_home: Encryption::None,
            users: vec![DraftUser {
                name: "admin".into(),
                shell: String::new(),
            }],
            ssh_keys: Vec::new(),
            services: Vec::new(),
        }
    }

    /// Re-derives the version default from the current source base:
    /// the newest release with a slot artifact there, or empty.
    /// Called when the operator edits the source base.
    pub fn refresh_version(&mut self) {
        self.version = source_version(&self.source_base);
    }

    /// Validates the draft the way the engine does: parse the raw
    /// fields, render the config to TOML, and re-parse it with the
    /// strict parser. Returns all diagnostics found in one pass; an
    /// `Err` always carries at least one.
    pub fn config(&self) -> Result<crate::config::Config, Vec<String>> {
        let mut errs: Vec<String> = Vec::new();
        let version = match crate::version::parse(&self.version) {
            Ok(v) => v,
            Err(e) => {
                errs.push(format!("[source] {e}"));
                Version::default()
            }
        };
        let mut size_of = |key: &str, s: &str| -> ByteSize {
            match crate::size::parse_size(s) {
                Ok(sz) if crate::size::is_mib_multiple(sz) => sz,
                Ok(_) => {
                    errs.push(format!(
                        "[partitions] key '{key}': size '{s}' must be a multiple of 1 MiB"
                    ));
                    ByteSize(0)
                }
                Err(e) => {
                    errs.push(format!("[partitions] key '{key}': {e}"));
                    ByteSize(0)
                }
            }
        };
        let cfg = crate::config::Config {
            target_disk: self.target_disk.clone(),
            source_base: self.source_base.clone(),
            version,
            hostname: self.hostname.clone(),
            timezone: self.timezone.clone(),
            locale: self.locale.clone(),
            keymap: self.keymap.clone(),
            esp: size_of("esp", &self.esp),
            slot_a: size_of("slot_a", &self.slot_a),
            slot_b: size_of("slot_b", &self.slot_b),
            var: size_of("var", &self.var),
            home: size_of("home", &self.home),
            fs_slot: self.fs_slot,
            fs_var: self.fs_var,
            fs_home: self.fs_home,
            enc_var: self.enc_var,
            enc_home: self.enc_home,
            users: self
                .users
                .iter()
                .map(|u| User {
                    name: u.name.clone(),
                    shell: if u.shell.is_empty() {
                        None
                    } else {
                        Some(u.shell.clone())
                    },
                })
                .collect(),
            ssh_keys: self.ssh_keys.clone(),
            services: self.services.clone(),
        };
        if !errs.is_empty() {
            return Err(errs);
        }
        // The rendered document must survive the engine's own strict
        // parse (unknown keys, category shapes, value validation).
        config::parse(&config::render(&cfg))
    }
}

/// The newest version with a slot artifact under `base` (relative
/// paths resolve against the current directory), else empty.
fn source_version(base: &str) -> String {
    crate::source::available_versions(Path::new(base))
        .into_iter()
        .next()
        .map(|v| v.to_string())
        .unwrap_or_default()
}

pub mod ui;

#[cfg(test)]
mod tests;
