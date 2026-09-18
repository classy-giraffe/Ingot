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
fn explicit_artifacts_mode_parses() {
    let doc = valid().replace(
        "[source]\nbase = \"dist\"\nversion = \"0.1.0\"\n",
        "[source]\nmode = \"artifacts\"\nbase = \"dist\"\nversion = \"0.1.0\"\n",
    );
    let c = parse(&doc).unwrap();
    assert!(matches!(c.source_mode, SourceMode::Artifacts));
    assert_eq!(c.source_base, "dist");
    assert_eq!(c.version, version::parse("0.1.0").unwrap());
}

#[test]
fn live_mode_parses_without_version() {
    let doc = valid().replace(
        "[source]\nbase = \"dist\"\nversion = \"0.1.0\"\n",
        "[source]\nmode = \"live\"\nbase = \"/media/ingot-iso\"\n",
    );
    let c = parse(&doc).unwrap();
    assert!(matches!(c.source_mode, SourceMode::Live));
    assert_eq!(c.source_base, "/media/ingot-iso");
}

#[test]
fn live_mode_rejects_version() {
    let doc = valid().replace(
        "[source]\nbase = \"dist\"\nversion = \"0.1.0\"\n",
        "[source]\nmode = \"live\"\nbase = \"/media/ingot-iso\"\nversion = \"0.1.0\"\n",
    );
    let errs = parse(&doc).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("[source]") && e.contains("version") && e.contains("live")),
        "expected a live-mode version diagnostic, got {errs:?}"
    );
}

#[test]
fn live_mode_requires_base() {
    let doc = valid().replace(
        "[source]\nbase = \"dist\"\nversion = \"0.1.0\"\n",
        "[source]\nmode = \"live\"\n",
    );
    let errs = parse(&doc).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("[source]") && e.contains("base")),
        "expected a missing-base diagnostic, got {errs:?}"
    );
}

#[test]
fn bad_source_mode_fails_named() {
    let doc = valid().replace(
        "[source]\nbase = \"dist\"\nversion = \"0.1.0\"\n",
        "[source]\nmode = \"usb\"\nbase = \"dist\"\nversion = \"0.1.0\"\n",
    );
    let errs = parse(&doc).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("[source]") && e.contains("mode") && e.contains("usb")),
        "expected a bad-mode diagnostic, got {errs:?}"
    );
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

// --- render: the wizard's output path ---

#[test]
fn render_matches_canonical_literal() {
    let cfg = parse(&valid()).unwrap();
    let expected = r#"schema = 1

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
"#;
    assert_eq!(render(&cfg), expected);
}

#[test]
fn render_round_trips_through_the_strict_parser() {
    let base = parse(&valid()).unwrap();
    assert_eq!(parse(&render(&base)).unwrap(), base);

    // Variants that exercise the non-default branches: ext4 state,
    // luks2 (accepted at parse; the engine's validation phase
    // rejects it in v1), two users (the second without a shell),
    // a MiB-only size (no exact GiB), an ssh key whose comment
    // needs quote escaping, a service, and a target path with an
    // embedded double quote.
    let mut c = base.clone();
    c.target_disk = r#"/tmp/di"sk.qcow2"#.to_string();
    c.fs_var = StateFs::Ext4;
    c.enc_home = Encryption::Luks2;
    c.home = ByteSize(1536 << 20);
    c.users.push(User {
        name: "ada".into(),
        shell: None,
    });
    c.ssh_keys
        .push("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF1aBsIu1m9uoJkCke9zOtv1hJwZG3pHe4PMYdgPQ+Bb \"quoted\"".into());
    c.services.push("sshd.service".into());
    let rt = parse(&render(&c)).unwrap();
    assert_eq!(rt, c);
}
