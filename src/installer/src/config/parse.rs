//! Per-category TOML parsing (spec 11.4): one function per
//! config category. Each function validates its category and
//! collects diagnostics into `errs` - a bad category never
//! aborts the parse, the full diagnostic set is reported at
//! once.

use super::*;

/// 11.4 category parse: target.
pub(super) fn parse_target(doc: &toml::Table, errs: &mut Vec<String>) -> String {
    // 11.4.1 target disk
    let target_disk = match doc.get("target") {
        None => {
            errs.push("category '[target]' missing (required: disk)".into());
            String::new()
        }
        Some(v) => match table(v, "target", errs) {
            None => String::new(),
            Some(t) => {
                unknown_keys(t, "target", &["disk"], errs);
                str_key(t, "target", "disk", errs)
            }
        },
    };
    if !target_disk.is_empty() && !target_disk.starts_with('/') {
        errs.push(format!(
            "[target] disk '{target_disk}' must be an absolute path (block device or image file)"
        ));
    }
    target_disk
}
/// 11.4 category parse: source.
///
/// `mode` selects the payload source: `artifacts` (the default)
/// deploys the prebuilt whole-image artifact set from `base` for the
/// given `version`; `live` (spec 10.2) runs the installer on the live
/// ISO, where `base` is the mounted ISO media carrying the running
/// release's erofs (LiveOS/rootfs.erofs) and the release artifacts
/// the install deploys (the installed UKI and the ESP tree under
/// esp/). Live mode takes no `version`: the running release is the
/// source release (the engine resolves it from the running
/// os-release).
pub(super) fn parse_source(
    doc: &toml::Table,
    errs: &mut Vec<String>,
) -> (SourceMode, String, Version) {
    // 11.4.2 OS image source
    let (mode, source_base, version) = match doc.get("source") {
        None => {
            errs.push(
                "category '[source]' missing (required: base, version; or mode = \"live\")".into(),
            );
            (SourceMode::Artifacts, String::new(), Version::default())
        }
        Some(v) => match table(v, "source", errs) {
            None => (SourceMode::Artifacts, String::new(), Version::default()),
            Some(t) => {
                unknown_keys(t, "source", &["mode", "base", "version"], errs);
                let mode = match t.get("mode") {
                    None => SourceMode::Artifacts,
                    Some(toml::Value::String(s)) => match s.as_str() {
                        "artifacts" => SourceMode::Artifacts,
                        "live" => SourceMode::Live,
                        other => {
                            errs.push(format!(
                                "[source] key 'mode' must be \"artifacts\" or \"live\" (got {other:?})"
                            ));
                            SourceMode::Artifacts
                        }
                    },
                    Some(v) => {
                        errs.push(format!("[source] key 'mode' must be a string ({v})"));
                        SourceMode::Artifacts
                    }
                };
                let base = str_key(t, "source", "base", errs);
                let v = match t.get("version") {
                    None if mode == SourceMode::Live => Version::default(),
                    None => {
                        errs.push("[source] key 'version' missing".into());
                        Version::default()
                    }
                    Some(toml::Value::String(s)) if mode == SourceMode::Live => {
                        errs.push(
                            "[source] key 'version' is not allowed in live mode (the running release is the source release)".into(),
                        );
                        Version::default()
                    }
                    Some(toml::Value::String(s)) => match version::parse(s.as_str()) {
                        Ok(v) => v,
                        Err(e) => {
                            errs.push(format!("[source] {e}"));
                            Version::default()
                        }
                    },
                    Some(v) => {
                        errs.push(format!("[source] key 'version' must be a string ({v})"));
                        Version::default()
                    }
                };
                (mode, base, v)
            }
        },
    };
    (mode, source_base, version)
}
/// 11.4 category parse: system.
pub(super) fn parse_system(
    doc: &toml::Table,
    errs: &mut Vec<String>,
) -> (String, String, String, String) {
    // 11.4.3-5 system identity
    let (hostname, timezone, locale, keymap) = match doc.get("system") {
        None => {
            errs.push(
                "category '[system]' missing (required: hostname, timezone, locale, keymap)".into(),
            );
            (String::new(), String::new(), String::new(), String::new())
        }
        Some(v) => match table(v, "system", errs) {
            None => (String::new(), String::new(), String::new(), String::new()),
            Some(t) => {
                unknown_keys(
                    t,
                    "system",
                    &["hostname", "timezone", "locale", "keymap"],
                    errs,
                );
                let hostname = str_key(t, "system", "hostname", errs);
                let timezone = str_key(t, "system", "timezone", errs);
                let locale = str_key(t, "system", "locale", errs);
                let keymap = str_key(t, "system", "keymap", errs);
                if hostname.is_empty() {
                    errs.push("[system] key 'hostname' must be a non-empty string".into());
                } else if !is_hostname(&hostname) {
                    errs.push(format!(
                        "[system] hostname '{hostname}' is invalid (1-64 chars, alphanumerics and hyphens, no leading/trailing hyphen)"
                    ));
                }
                for (name, value) in [
                    ("timezone", &timezone),
                    ("locale", &locale),
                    ("keymap", &keymap),
                ] {
                    if value.is_empty() {
                        errs.push(format!("[system] key '{name}' must be a non-empty string"));
                    }
                }
                (hostname, timezone, locale, keymap)
            }
        },
    };
    (hostname, timezone, locale, keymap)
}
/// 11.4 category parse: partitions.
pub(super) fn parse_partitions(
    doc: &toml::Table,
    errs: &mut Vec<String>,
) -> (ByteSize, ByteSize, ByteSize, ByteSize, ByteSize) {
    // 11.4.6 partition sizes
    let (esp, slot_a, slot_b, var, home) = match doc.get("partitions") {
        None => {
            errs.push(
                "category '[partitions]' missing (required: esp, slot_a, slot_b, var, home)".into(),
            );
            (
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
            )
        }
        Some(v) => match table(v, "partitions", errs) {
            None => (
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
                ByteSize(0),
            ),
            Some(t) => {
                unknown_keys(
                    t,
                    "partitions",
                    &["esp", "slot_a", "slot_b", "var", "home"],
                    errs,
                );
                (
                    size_key(t, "partitions", "esp", errs),
                    size_key(t, "partitions", "slot_a", errs),
                    size_key(t, "partitions", "slot_b", errs),
                    size_key(t, "partitions", "var", errs),
                    size_key(t, "partitions", "home", errs),
                )
            }
        },
    };
    (esp, slot_a, slot_b, var, home)
}
/// 11.4 category parse: filesystems.
pub(super) fn parse_filesystems(
    doc: &toml::Table,
    errs: &mut Vec<String>,
) -> (SlotFs, StateFs, StateFs) {
    // 11.4.7 filesystem choices
    let (fs_slot, fs_var, fs_home) = match doc.get("filesystems") {
        None => {
            errs.push("category '[filesystems]' missing (required: slot, var, home)".into());
            (SlotFs::Eros, StateFs::Btrfs, StateFs::Btrfs)
        }
        Some(v) => match table(v, "filesystems", errs) {
            None => (SlotFs::Eros, StateFs::Btrfs, StateFs::Btrfs),
            Some(t) => {
                unknown_keys(t, "filesystems", &["slot", "var", "home"], errs);
                let fs_slot = match t.get("slot") {
                    None => {
                        errs.push("[filesystems] key 'slot' missing".into());
                        SlotFs::Eros
                    }
                    Some(toml::Value::String(s)) => match s.as_str() {
                        "erofs" => SlotFs::Eros,
                        other => {
                            errs.push(format!(
                                "[filesystems] slot '{other}' is not supported in v1 (expected 'erofs')"
                            ));
                            SlotFs::Eros
                        }
                    },
                    Some(v) => {
                        errs.push(format!("[filesystems] key 'slot' must be a string ({v})"));
                        SlotFs::Eros
                    }
                };
                let fs_var = state_fs_key(t, "filesystems", "var", errs);
                let fs_home = state_fs_key(t, "filesystems", "home", errs);
                (fs_slot, fs_var, fs_home)
            }
        },
    };
    (fs_slot, fs_var, fs_home)
}
/// 11.4 category parse: encryption.
pub(super) fn parse_encryption(
    doc: &toml::Table,
    errs: &mut Vec<String>,
) -> (Encryption, Encryption) {
    // 11.4.8 encryption choices
    let (enc_var, enc_home) = match doc.get("encryption") {
        None => {
            errs.push("category '[encryption]' missing (required: var, home)".into());
            (Encryption::None, Encryption::None)
        }
        Some(v) => match table(v, "encryption", errs) {
            None => (Encryption::None, Encryption::None),
            Some(t) => {
                unknown_keys(t, "encryption", &["var", "home"], errs);
                (
                    enc_key(t, "encryption", "var", errs),
                    enc_key(t, "encryption", "home", errs),
                )
            }
        },
    };
    (enc_var, enc_home)
}
/// 11.4 category parse: users.
pub(super) fn parse_users(doc: &toml::Table, errs: &mut Vec<String>) -> Vec<User> {
    // 11.4.9 initial users
    
    match doc.get("users") {
        None => {
            errs.push(
                "category '[[users]]' missing (at least one initial user is required)".into(),
            );
            Vec::new()
        }
        Some(toml::Value::Array(items)) => {
            if items.is_empty() {
                errs.push("category '[[users]]' must list at least one initial user".into());
            }
            let mut users: Vec<User> = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let what = format!("[[users]] entry {i}");
                let item = match item.as_table() {
                    Some(t) => t,
                    None => {
                        errs.push(format!("{what} must be a table"));
                        continue;
                    }
                };
                unknown_keys(item, &what, &["name", "shell"], errs);
                let name = match item.get("name") {
                    None => {
                        errs.push(format!("{what} key 'name' missing"));
                        continue;
                    }
                    Some(toml::Value::String(s)) => s.clone(),
                    Some(v) => {
                        errs.push(format!("{what} key 'name' must be a string ({v})"));
                        continue;
                    }
                };
                if !is_user_name(&name) {
                    errs.push(format!(
                        "{what} name '{name}' is invalid (lowercase letters, digits, '_', '-' or '.'; must start with a letter or '_'; max 32 chars)"
                    ));
                }
                let shell = match item.get("shell") {
                    None => None,
                    Some(toml::Value::String(s)) => {
                        if !s.starts_with('/') {
                            errs.push(format!("{what} shell '{s}' must be an absolute path"));
                        }
                        Some(s.clone())
                    }
                    Some(v) => {
                        errs.push(format!("{what} key 'shell' must be a string ({v})"));
                        None
                    }
                };
                if users.iter().any(|u| u.name == name) {
                    errs.push(format!("{what} name '{name}' is duplicated"));
                }
                users.push(User { name, shell });
            }
            users
        }
        Some(v) => {
            errs.push(format!(
                "category '[[users]]' must be an array of tables ({v})"
            ));
            Vec::new()
        }
    }
}
/// 11.4 category parse: ssh.
pub(super) fn parse_ssh(doc: &toml::Table, errs: &mut Vec<String>) -> Vec<String> {
    // 11.4.10 SSH authorized keys
    
    match doc.get("ssh") {
        None => {
            errs.push("category '[ssh]' missing (required: authorized_keys)".into());
            Vec::new()
        }
        Some(v) => match table(v, "ssh", errs) {
            None => Vec::new(),
            Some(t) => {
                unknown_keys(t, "ssh", &["authorized_keys"], errs);
                
                match t.get("authorized_keys") {
                    None => {
                        errs.push("[ssh] key 'authorized_keys' missing".into());
                        Vec::new()
                    }
                    Some(toml::Value::Array(items)) => {
                        let mut keys = Vec::new();
                        for (i, item) in items.iter().enumerate() {
                            let what = format!("[ssh] authorized_keys entry {i}");
                            let item = match item.as_str() {
                                Some(s) => s,
                                None => {
                                    errs.push(format!("{what} must be a string"));
                                    continue;
                                }
                            };
                            match sshkey::validate(item) {
                                Ok(()) => keys.push(item.to_string()),
                                Err(e) => errs.push(format!("{what}: {e}")),
                            }
                        }
                        keys
                    }
                    Some(v) => {
                        errs.push(format!(
                            "[ssh] key 'authorized_keys' must be an array of strings ({v})"
                        ));
                        Vec::new()
                    }
                }
            }
        },
    }
}
/// 11.4 category parse: services.
pub(super) fn parse_services(doc: &toml::Table, errs: &mut Vec<String>) -> Vec<String> {
    // 11.4.11 service enablement policy
    
    match doc.get("services") {
        None => {
            errs.push("category '[services]' missing (required: enabled)".into());
            Vec::new()
        }
        Some(v) => match table(v, "services", errs) {
            None => Vec::new(),
            Some(t) => {
                unknown_keys(t, "services", &["enabled"], errs);
                
                match t.get("enabled") {
                    None => {
                        errs.push("[services] key 'enabled' missing".into());
                        Vec::new()
                    }
                    Some(toml::Value::Array(items)) => {
                        let mut units = Vec::new();
                        for (i, item) in items.iter().enumerate() {
                            let what = format!("[services] enabled entry {i}");
                            let item = match item.as_str() {
                                Some(s) => s,
                                None => {
                                    errs.push(format!("{what} must be a string"));
                                    continue;
                                }
                            };
                            if !is_unit_name(item) {
                                errs.push(format!("{what} '{item}' is not a valid unit name"));
                            }
                            units.push(item.to_string());
                        }
                        units
                    }
                    Some(v) => {
                        errs.push(format!(
                            "[services] key 'enabled' must be an array of strings ({v})"
                        ));
                        Vec::new()
                    }
                }
            }
        },
    }
}
