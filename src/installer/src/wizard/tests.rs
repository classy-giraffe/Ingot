use super::*;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingot-wiz-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn defaults_match_the_workstation_profile() {
    let d = Draft::new();
    // Target disk is auto-probed from /sys/block (starts with /dev/ or empty if none).
    assert!(d.target_disk.is_empty() || d.target_disk.starts_with("/dev/"));
    assert_eq!(d.source_base, "dist");
    // The test runs with the crate directory as CWD, which carries
    // no dist/: no discoverable version.
    assert!(d.version.is_empty());
    // System identity (11.4.3-5).
    assert_eq!(d.hostname, "ingot");
    assert_eq!(d.timezone, "UTC");
    assert_eq!(d.locale, "C.UTF-8");
    assert_eq!(d.keymap, "us");
    // Partition sizes (11.4.6): the 1+8+8+4+8 GiB baseline layout.
    assert_eq!(d.esp, "1G");
    assert_eq!(d.slot_a, "8G");
    assert_eq!(d.slot_b, "8G");
    assert_eq!(d.var, "4G");
    assert_eq!(d.home, "8G");
    // Filesystem and encryption choices (11.4.7-8): the v1 set.
    assert_eq!(d.fs_slot, SlotFs::Eros);
    assert_eq!(d.fs_var, StateFs::Btrfs);
    assert_eq!(d.fs_home, StateFs::Btrfs);
    assert_eq!(d.enc_var, Encryption::None);
    assert_eq!(d.enc_home, Encryption::None);
    // One initial user (11.4.9), the default shell, no keys (11.4.10),
    // no extra services (11.4.11).
    assert_eq!(d.users, vec![DraftUser { name: "admin".into(), shell: String::new() }]);
    assert!(d.ssh_keys.is_empty());
    assert!(d.services.is_empty());
}

#[test]
fn refresh_version_picks_the_newest_available() {
    let d = tmp("refresh");
    for v in ["0.1.0", "0.2.0"] {
        std::fs::write(d.join(format!("ingot_{v}.slot.raw")), b"x").unwrap();
    }
    let mut dr = Draft::new();
    dr.source_base = d.to_string_lossy().to_string();
    dr.refresh_version();
    assert_eq!(dr.version, "0.2.0");
    dr.source_base = d.join("no-such-dir").to_string_lossy().to_string();
    dr.refresh_version();
    assert!(dr.version.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn config_validates_and_parses_the_default_draft() {
    let mut d = Draft::new();
    d.target_disk = "/dev/vda".into();
    d.version = "0.1.0".into();
    let cfg = d.config().unwrap();
    assert_eq!(cfg.target_disk, "/dev/vda");
    assert_eq!(cfg.hostname, "ingot");
    assert_eq!(cfg.esp, ByteSize(1 << 30));
    assert_eq!(cfg.home, ByteSize(8 << 30));
    assert_eq!(
        cfg.users,
        vec![crate::config::User {
            name: "admin".into(),
            shell: None,
        }]
    );
    // The rendered draft round-trips: this is the single execution
    // path the engine re-parses.
    let reparsed = crate::config::parse(&crate::config::render(&cfg)).unwrap();
    assert_eq!(reparsed, cfg);
}

#[test]
fn config_detects_live_source_mode_and_roundtrips() {
    let dir = tmp("live-draft");
    let erofs = dir.join(crate::source::LIVE_EROFS);
    std::fs::create_dir_all(erofs.parent().unwrap()).unwrap();
    std::fs::write(&erofs, b"fake-erofs").unwrap();

    let mut d = Draft::new();
    d.target_disk = "/dev/vda".into();
    d.source_base = dir.to_string_lossy().to_string();
    d.version = "0.1.0".into();

    let cfg = d.config().expect("live draft should validate cleanly");
    assert_eq!(cfg.source_mode, crate::config::SourceMode::Live);
    assert_eq!(cfg.source_base, dir.to_string_lossy());

    let rendered = crate::config::render(&cfg);
    assert!(rendered.contains("mode = \"live\""));
    assert!(!rendered.contains("version ="));

    let reparsed = crate::config::parse(&rendered).unwrap();
    assert_eq!(reparsed.source_mode, crate::config::SourceMode::Live);
    assert_eq!(reparsed.source_base, dir.to_string_lossy());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_diagnostics_name_the_category_and_key() {
    let mut d = Draft::new();
    d.target_disk = "/dev/vda".into();
    d.version = "0.1.0".into();

    // Bad size -> the engine's own naming.
    d.esp = "nope".into();
    let errs = d.config().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("[partitions] key 'esp'")), "{errs:?}");

    // Bad version.
    let mut d = Draft::new();
    d.version = "1.2".into();
    let errs = d.config().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("[source]")), "{errs:?}");

    // Reserved user name (engine's diagnostic).
    let mut d = Draft::new();
    d.target_disk = "/dev/vda".into();
    d.version = "0.1.0".into();
    d.users = vec![DraftUser {
        name: "root".into(),
        shell: String::new(),
    }];
    let errs = d.config().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("[[users]] entry 0")), "{errs:?}");

    // Relative target path (engine's diagnostic).
    let mut d = Draft::new();
    d.target_disk = "disk.qcow2".into();
    d.version = "0.1.0".into();
    let errs = d.config().unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("[target] disk 'disk.qcow2' must be an absolute path")),
        "{errs:?}"
    );

    // Bad ssh key (engine's diagnostic).
    let mut d = Draft::new();
    d.target_disk = "/dev/vda".into();
    d.version = "0.1.0".into();
    d.ssh_keys.push("ssh-ed25519 not-base64 host".into());
    let errs = d.config().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("[ssh]")), "{errs:?}");

    // Diagnostics are collected in one pass, not cut off at the first.
    let mut d = Draft::new();
    d.esp = "nope".into();
    d.var = "alsobad".into();
    let errs = d.config().unwrap_err();
    assert!(errs.len() >= 2, "{errs:?}");
}

#[test]
fn step_navigation_clamps_at_the_ends() {
    // The 11.4 categories in order, then the review.
    assert_eq!(
        Step::ALL,
        [
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
        ]
    );
    assert_eq!(Step::Target.prev(), None);
    assert_eq!(Step::Review.next(), None);
    assert_eq!(Step::Target.next(), Some(Step::Source));
    assert_eq!(Step::Services.next(), Some(Step::Review));
    assert_eq!(Step::Review.prev(), Some(Step::Services));
    // Walking next from the first step visits every screen once.
    let mut s = Step::Target;
    let mut seen = vec![s];
    while let Some(n) = s.next() {
        seen.push(n);
        s = n;
    }
    assert_eq!(seen, Step::ALL.to_vec());
    // The review is the only screen with a "continue" beyond it.
    assert!(Step::ALL[..9].iter().all(|s| s.next().is_some()));
}
