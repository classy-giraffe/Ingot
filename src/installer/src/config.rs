//! Declarative install config (spec 11.4): the TOML document that
//! authorizes and drives an install.
//!
//! All eleven spec-11.4 input categories are REQUIRED; a missing or
//! invalid category fails with a diagnostic that names the category
//! and the key. Parsing is strict: unknown keys at the top level or
//! in any category are errors, so a config cannot silently carry
//! categories the engine does not understand.
//!
//! The schema version is `schema = 1` at the top level.

use crate::{size, sshkey, version};
use size::ByteSize;
use version::Version;

mod parse;
use parse::*;

/// Default login shell for initial users (spec: nushell is the
/// default interactive shell). The shell must exist in the slot; the
/// validation phase checks it against the installed release.
pub const DEFAULT_SHELL: &str = "/usr/bin/nushell";

/// The parsed and validated install config.
#[derive(Debug, Clone)]
pub struct Config {
    // 11.4.1 target disk
    pub target_disk: String,
    // 11.4.2 OS image source
    pub source_base: String,
    pub version: Version,
    // 11.4.3-5 system identity
    pub hostname: String,
    pub timezone: String,
    pub locale: String,
    pub keymap: String,
    // 11.4.6 partition sizes
    pub esp: ByteSize,
    pub slot_a: ByteSize,
    pub slot_b: ByteSize,
    pub var: ByteSize,
    pub home: ByteSize,
    // 11.4.7 filesystem choices
    pub fs_slot: SlotFs,
    pub fs_var: StateFs,
    pub fs_home: StateFs,
    // 11.4.8 encryption choices
    pub enc_var: Encryption,
    pub enc_home: Encryption,
    // 11.4.9 initial user configuration
    pub users: Vec<User>,
    // 11.4.10 SSH authorized keys
    pub ssh_keys: Vec<String>,
    // 11.4.11 service enablement policy
    pub services: Vec<String>,
}

/// OS payload slot filesystem (v1: erofs only; ext4 stays an
/// unimplemented spec MAY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotFs {
    Eros,
}

/// State partition filesystem (spec 8.4: btrfs or ext4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFs {
    Btrfs,
    Ext4,
}

/// Partition encryption (v1: unencrypted; LUKS2 is deferred fog and
/// the validation phase rejects it with a clear diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    None,
    Luks2,
}

/// An initial user (11.4.9). The first user is the administrator
/// (uid 1000); later users take the next free uids.
#[derive(Debug, Clone)]
pub struct User {
    pub name: String,
    /// Login shell; `None` = DEFAULT_SHELL.
    pub shell: Option<String>,
}

/// Parses and validates an install config.
///
/// Returns ALL diagnostics found (not just the first) so a bad
/// config can be fixed in one pass; an empty error list is impossible
/// on `Err` (at least one diagnostic is always present).
pub fn parse(text: &str) -> Result<Config, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let doc: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(vec![format!("config is not valid TOML: {e}")]),
    };
    let doc = match doc.as_table() {
        Some(t) => t,
        None => return Err(vec!["config must be a TOML table".into()]),
    };

    // schema: required integer 1.
    match doc.get("schema") {
        None => errs.push("top-level key 'schema' missing (expected 1)".into()),
        Some(toml::Value::Integer(1)) => {}
        Some(v) => errs.push(format!("unsupported schema {v} (expected 1)")),
    }

    // Strict top level: exactly {schema, target, source, system,
    // partitions, filesystems, encryption, users, ssh, services}.
    const CATS: [&str; 9] = [
        "target",
        "source",
        "system",
        "partitions",
        "filesystems",
        "encryption",
        "users",
        "ssh",
        "services",
    ];
    for key in doc.keys() {
        if key != "schema" && !CATS.contains(&key.as_str()) {
            errs.push(format!("unknown top-level key '{key}'"));
        }
    }

    let target_disk = parse_target(doc, &mut errs);
    let (source_base, version) = parse_source(doc, &mut errs);
    let (hostname, timezone, locale, keymap) = parse_system(doc, &mut errs);
    let (esp, slot_a, slot_b, var, home) = parse_partitions(doc, &mut errs);
    let (fs_slot, fs_var, fs_home) = parse_filesystems(doc, &mut errs);
    let (enc_var, enc_home) = parse_encryption(doc, &mut errs);
    let users = parse_users(doc, &mut errs);
    let ssh_keys = parse_ssh(doc, &mut errs);
    let services = parse_services(doc, &mut errs);

    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(Config {
        target_disk,
        source_base,
        version,
        hostname,
        timezone,
        locale,
        keymap,
        esp,
        slot_a,
        slot_b,
        var,
        home,
        fs_slot,
        fs_var,
        fs_home,
        enc_var,
        enc_home,
        users,
        ssh_keys,
        services,
    })
}

// ---------------------------------------------------------------- helpers ---

/// Borrows a category value as a table; a non-table value gets a
/// named diagnostic and yields None (collection continues).
fn table<'a>(v: &'a toml::Value, cat: &str, errs: &mut Vec<String>) -> Option<&'a toml::Table> {
    match v.as_table() {
        Some(t) => Some(t),
        None => {
            errs.push(format!("[{cat}] must be a table"));
            None
        }
    }
}

/// System accounts the install config must not shadow.
const RESERVED_USER_NAMES: [&str; 16] = [
    "root",
    "bin",
    "daemon",
    "adm",
    "lp",
    "sync",
    "mail",
    "news",
    "uucp",
    "operator",
    "games",
    "ftp",
    "nobody",
    "nobody4",
    "systemd-network",
    "tss",
];

/// A valid user name: useradd-compatible, lowercase, not a reserved
/// system account.
pub(crate) fn is_user_name(name: &str) -> bool {
    if RESERVED_USER_NAMES.contains(&name) {
        return false;
    }
    let b = name.as_bytes();
    if !(1..=32).contains(&b.len()) {
        return false;
    }
    let first_ok = b[0].is_ascii_lowercase() || b[0] == b'_';
    first_ok
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
}

/// A valid systemd unit name (v1 suffixes).
pub(crate) fn is_unit_name(name: &str) -> bool {
    const SUFFIXES: [&str; 5] = [".service", ".socket", ".path", ".timer", ".target"];
    (1..=255).contains(&name.len())
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b':' | b'_' | b'@' | b'-'))
        && !name.starts_with('.')
        && SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// 1-64 chars, alphanumerics and hyphens, no leading/trailing hyphen.
fn is_hostname(h: &str) -> bool {
    (1..=64).contains(&h.len())
        && !h.starts_with('-')
        && !h.ends_with('-')
        && h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// Collects a required string key; pushes a diagnostic on missing or
/// wrong type. Returns "" when the key is unusable.
fn str_key(t: &toml::Table, cat: &str, key: &str, errs: &mut Vec<String>) -> String {
    match t.get(key) {
        None => {
            errs.push(format!("[{cat}] key '{key}' missing"));
            String::new()
        }
        Some(toml::Value::String(s)) => s.clone(),
        Some(v) => {
            errs.push(format!("[{cat}] key '{key}' must be a string ({v})"));
            String::new()
        }
    }
}

/// Collects a required size key (byte count, 1 MiB multiple).
fn size_key(t: &toml::Table, cat: &str, key: &str, errs: &mut Vec<String>) -> ByteSize {
    let v = match t.get(key) {
        None => {
            errs.push(format!("[{cat}] key '{key}' missing"));
            return ByteSize(0);
        }
        Some(v) => v,
    };
    let s = match v.as_str() {
        Some(s) => s,
        None => {
            errs.push(format!(
                "[{cat}] key '{key}' must be a string (a size like '8G')"
            ));
            return ByteSize(0);
        }
    };
    match size::parse_size(s) {
        Ok(sz) if size::is_mib_multiple(sz) => sz,
        Ok(_) => {
            errs.push(format!(
                "[{cat}] key '{key}': size '{s}' must be a multiple of 1 MiB"
            ));
            ByteSize(0)
        }
        Err(e) => {
            errs.push(format!("[{cat}] key '{key}': {e}"));
            ByteSize(0)
        }
    }
}

fn state_fs_key(t: &toml::Table, cat: &str, key: &str, errs: &mut Vec<String>) -> StateFs {
    match t.get(key) {
        None => {
            errs.push(format!("[{cat}] key '{key}' missing"));
            StateFs::Btrfs
        }
        Some(toml::Value::String(s)) => match s.as_str() {
            "btrfs" => StateFs::Btrfs,
            "ext4" => StateFs::Ext4,
            other => {
                errs.push(format!(
                    "[{cat}] key '{key}': '{other}' is not supported (expected 'btrfs' or 'ext4')"
                ));
                StateFs::Btrfs
            }
        },
        Some(v) => {
            errs.push(format!("[{cat}] key '{key}' must be a string ({v})"));
            StateFs::Btrfs
        }
    }
}

fn enc_key(t: &toml::Table, cat: &str, key: &str, errs: &mut Vec<String>) -> Encryption {
    match t.get(key) {
        None => {
            errs.push(format!("[{cat}] key '{key}' missing"));
            Encryption::None
        }
        Some(toml::Value::String(s)) => match s.as_str() {
            "none" => Encryption::None,
            "luks2" => Encryption::Luks2,
            other => {
                errs.push(format!(
                    "[{cat}] key '{key}': '{other}' is not a known encryption choice (expected 'none' or 'luks2')"
                ));
                Encryption::None
            }
        },
        Some(v) => {
            errs.push(format!("[{cat}] key '{key}' must be a string ({v})"));
            Encryption::None
        }
    }
}

/// Reports keys present in a category table that are not in the
/// allowed set.
fn unknown_keys(t: &toml::Table, cat: &str, allowed: &[&str], errs: &mut Vec<String>) {
    for key in t.keys() {
        if !allowed.contains(&key.as_str()) {
            errs.push(format!("[{cat}] unknown key '{key}'"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_names_reserved_and_shape() {
        assert!(is_user_name("tommy"));
        assert!(is_user_name("a-b_c9"));
        assert!(!is_user_name("Tommy"));
        assert!(!is_user_name(""));
        assert!(!is_user_name("bad:name"));
        assert!(!is_user_name("root"));
        assert!(!is_user_name("nobody"));
        assert!(!is_user_name("systemd-network"));
    }

    #[test]
    fn unit_names_v1_suffixes() {
        assert!(is_unit_name("sshd.service"));
        assert!(is_unit_name("ssh@server.service"));
        assert!(!is_unit_name("no-dot"));
        assert!(!is_unit_name(".hidden.service"));
        assert!(!is_unit_name("bad;unit.service"));
    }

    /// A minimal valid config with all 11.4 categories.
    fn valid() -> String {
        r#"
schema = 1

[target]
disk = "/dev/vda"

[source]
base = "dist"
version = "0.1.0"

[system]
hostname = "ingot"
timezone = "UTC"
locale = "C.UTF-8"
keymap = "us"

[partitions]
esp = "1G"
slot_a = "8G"
slot_b = "8G"
var = "4G"
home = "8G"

[filesystems]
slot = "erofs"
var = "btrfs"
home = "btrfs"

[encryption]
var = "none"
home = "none"

[[users]]
name = "tommy"
shell = "/usr/bin/brush"

[ssh]
authorized_keys = []

[services]
enabled = []
"#
        .to_string()
    }

    /// Deletes the given top-level category: its header plus its keys,
    /// up to the next table header line (or end of document).
    fn without(doc: &str, cat: &str) -> String {
        let start = format!("\n[{cat}]\n");
        let s = doc.find(&start).expect("category anchor");
        let rest = &doc[s + start.len()..];
        let mut e = 0;
        for l in rest.lines() {
            if l.starts_with('[') {
                break;
            }
            e += l.len() + 1;
        }
        format!("{}{}", &doc[..s], &rest[e..])
    }

    #[test]
    fn valid_config_parses() {
        let c = parse(&valid()).unwrap();
        assert_eq!(c.target_disk, "/dev/vda");
        assert_eq!(c.source_base, "dist");
        assert_eq!(c.version, version::parse("0.1.0").unwrap());
        assert_eq!(c.hostname, "ingot");
        assert_eq!(c.esp, ByteSize(1 << 30));
        assert_eq!(c.slot_a, ByteSize(8 << 30));
        assert_eq!(c.var, ByteSize(4 << 30));
        assert!(matches!(c.fs_slot, SlotFs::Eros));
        assert!(matches!(c.fs_var, StateFs::Btrfs));
        assert!(matches!(c.enc_home, Encryption::None));
        assert_eq!(c.users.len(), 1);
        assert_eq!(c.users[0].name, "tommy");
        assert_eq!(c.services.len(), 0);
    }

    #[test]
    fn every_missing_category_fails_named() {
        for cat in [
            "target",
            "source",
            "system",
            "partitions",
            "filesystems",
            "encryption",
            "ssh",
            "services",
        ] {
            let doc = without(&valid(), cat);
            let errs = parse(&doc).unwrap_err();
            assert!(
                errs.iter()
                    .any(|e| e.contains(&format!("[{cat}]")) && e.contains("missing")),
                "expected a missing diagnostic naming [{cat}], got {errs:?}"
            );
        }
        // [[users]] is an array of tables, not a [table]
        let doc = valid().replace(
            "[[users]]\nname = \"tommy\"\nshell = \"/usr/bin/brush\"\n",
            "",
        );
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("[[users]]") && e.contains("missing")),
            "expected a missing diagnostic for [[users]], got {errs:?}"
        );
    }

    #[test]
    fn missing_schema_fails_named() {
        let doc = valid().replacen("schema = 1\n", "", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("'schema' missing")),
            "{errs:?}"
        );
    }

    #[test]
    fn unsupported_schema_fails() {
        let doc = valid().replacen("schema = 1", "schema = 2", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("unsupported schema")),
            "{errs:?}"
        );
    }

    #[test]
    fn unknown_keys_are_errors() {
        let doc = valid().replacen(
            "[target]\ndisk = \"/dev/vda\"",
            "[target]\ndisk = \"/dev/vda\"\nextra = 1",
            1,
        );
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("[target] unknown key 'extra'")),
            "{errs:?}"
        );
        let doc = valid().replacen("schema = 1\n", "schema = 1\nbogus = 1\n", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("unknown top-level key 'bogus'")),
            "{errs:?}"
        );
    }

    #[test]
    fn bad_values_fail_named() {
        // target disk not absolute
        let doc = valid().replacen("disk = \"/dev/vda\"", "disk = \"vda\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("[target] disk")), "{errs:?}");

        // bad version
        let doc = valid().replacen("version = \"0.1.0\"", "version = \"v0.1.0\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("[source]")), "{errs:?}");

        // bad hostname
        let doc = valid().replacen("hostname = \"ingot\"", "hostname = \"-x\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("hostname")), "{errs:?}");

        // empty timezone
        let doc = valid().replacen("timezone = \"UTC\"", "timezone = \"\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("timezone")), "{errs:?}");

        // size not a MiB multiple
        let doc = valid().replacen("esp = \"1G\"", "esp = \"500K\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("multiple of 1 MiB")),
            "{errs:?}"
        );

        // bad slot fs
        let doc = valid().replacen("slot = \"erofs\"", "slot = \"ext4\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("[filesystems] slot 'ext4'")),
            "{errs:?}"
        );

        // unknown encryption
        let doc = valid().replacen("var = \"none\"", "var = \"luks1\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("[encryption] key 'var'")),
            "{errs:?}"
        );

        // bad user name
        let doc = valid().replacen("name = \"tommy\"", "name = \"Tommy\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("invalid")), "{errs:?}");

        // non-absolute shell
        let doc = valid().replacen("shell = \"/usr/bin/brush\"", "shell = \"brush\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("absolute path")), "{errs:?}");

        // empty users
        let doc = valid().replacen(
            "[[users]]\nname = \"tommy\"\nshell = \"/usr/bin/brush\"",
            "[[users]]\n",
            1,
        );
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("[[users]]")), "{errs:?}");

        // bad ssh key
        let doc = valid().replacen(
            "authorized_keys = []",
            "authorized_keys = [\"ssh-ed25519 not-base64 host\"]",
            1,
        );
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("[ssh]")), "{errs:?}");

        // bad unit name
        let doc = valid().replacen("enabled = []", "enabled = [\"sshd\"]", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unit name")), "{errs:?}");

        // not TOML
        let errs = parse("not toml at all =").unwrap_err();
        assert!(errs.iter().any(|e| e.contains("TOML")), "{errs:?}");
    }

    #[test]
    fn all_diagnostics_are_collected() {
        // two independent errors in one document
        let doc = valid()
            .replacen("hostname = \"ingot\"", "hostname = \"\"", 1)
            .replacen("esp = \"1G\"", "esp = \"nope\"", 1);
        let errs = parse(&doc).unwrap_err();
        assert!(errs.len() >= 2, "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("hostname")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("esp")), "{errs:?}");
    }
}
