//! TOML rendering of a validated install config (spec 11.4).
//!
//! The wizard's output path: collect a draft, render it to TOML,
//! write the file, and let the engine re-read and re-parse it with
//! the same strict parser - one execution path for interactive and
//! declarative installs. The format is the canonical config layout:
//! `schema` first, then one table per category in spec 11.4 order,
//! sizes in the largest exact binary unit.

use super::{Config, Encryption, SlotFs, StateFs};
use crate::size::{ByteSize, MIB};

/// One gibibyte.
const GIB: u64 = MIB * 1024;

/// Renders `cfg` as the canonical TOML document.
pub fn render(cfg: &Config) -> String {
    let mut s = String::new();
    s.push_str("schema = 1\n\n");
    s.push_str("[target]\n");
    s.push_str(&format!("disk = {}\n\n", quote(&cfg.target_disk)));
    s.push_str("[source]\n");
    s.push_str(&format!(
        "base = {}\nversion = {}\n\n",
        quote(&cfg.source_base),
        quote(&cfg.version.to_string())
    ));
    s.push_str("[system]\n");
    s.push_str(&format!(
        "hostname = {}\ntimezone = {}\nlocale = {}\nkeymap = {}\n\n",
        quote(&cfg.hostname),
        quote(&cfg.timezone),
        quote(&cfg.locale),
        quote(&cfg.keymap)
    ));
    s.push_str("[partitions]\n");
    s.push_str(&format!(
        "esp = {}\nslot_a = {}\nslot_b = {}\nvar = {}\nhome = {}\n\n",
        quote(&size_str(cfg.esp)),
        quote(&size_str(cfg.slot_a)),
        quote(&size_str(cfg.slot_b)),
        quote(&size_str(cfg.var)),
        quote(&size_str(cfg.home))
    ));
    s.push_str("[filesystems]\n");
    s.push_str(&format!(
        "slot = {}\nvar = {}\nhome = {}\n\n",
        quote(fs_str(cfg.fs_slot)),
        quote(state_fs_str(cfg.fs_var)),
        quote(state_fs_str(cfg.fs_home))
    ));
    s.push_str("[encryption]\n");
    s.push_str(&format!(
        "var = {}\nhome = {}\n\n",
        quote(enc_str(cfg.enc_var)),
        quote(enc_str(cfg.enc_home))
    ));
    for u in &cfg.users {
        s.push_str("[[users]]\n");
        s.push_str(&format!("name = {}\n", quote(&u.name)));
        if let Some(shell) = &u.shell {
            s.push_str(&format!("shell = {}\n", quote(shell)));
        }
        s.push('\n');
    }
    s.push_str("[ssh]\n");
    s.push_str(&format!("authorized_keys = {}\n\n", str_array(&cfg.ssh_keys)));
    s.push_str("[services]\n");
    s.push_str(&format!("enabled = {}\n", str_array(&cfg.services)));
    s
}

/// A size in the largest exact binary unit: `8G`, `512M`, else the
/// bare byte count. Round-trips through `size::parse_size`.
fn size_str(s: ByteSize) -> String {
    let b = s.0;
    if b % GIB == 0 {
        format!("{}G", b / GIB)
    } else if b % MIB == 0 {
        format!("{}M", b / MIB)
    } else {
        b.to_string()
    }
}

fn fs_str(fs: SlotFs) -> &'static str {
    match fs {
        SlotFs::Eros => "erofs",
    }
}

fn state_fs_str(fs: StateFs) -> &'static str {
    match fs {
        StateFs::Btrfs => "btrfs",
        StateFs::Ext4 => "ext4",
    }
}

fn enc_str(e: Encryption) -> &'static str {
    match e {
        Encryption::None => "none",
        Encryption::Luks2 => "luks2",
    }
}

/// A TOML array of strings: `[]` or `['a', 'b']`.
fn str_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| quote(s)).collect();
    format!("[{}]", inner.join(", "))
}

/// A TOML basic string with minimal escaping.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
