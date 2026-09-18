use super::*;
#[test]
fn missing_tools_reports_gaps() {
    let dir = std::env::temp_dir().join("ingot-tools-test");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("systemd-repart"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(
        &dir.join("systemd-repart"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let missing = missing_tools(&dir.to_string_lossy(), &["mkfs.erofs".to_string()]);
    assert!(
        !missing.contains(&"systemd-repart".to_string()),
        "present tool reported missing: {missing:?}"
    );
    assert!(
        missing.contains(&"dd".to_string()),
        "absent tool not reported: {missing:?}"
    );
    assert!(
        missing.contains(&"mkfs.erofs".to_string()),
        "missing mkfs not reported: {missing:?}"
    );
    let _ = fs::remove_dir(&dir);
}
fn sample_cfg() -> Config {
    crate::config::parse(
            r#"
schema = 1
[target]
disk = "/tmp/does-not-matter-for-report"
[source]
base = "/tmp/does-not-matter"
version = "0.1.0"
[system]
hostname = "ingot1"
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
authorized_keys = ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= tommy@ingot"]
[services]
enabled = ["sshd.service"]
"#,
        )
        .expect("sample config parses")
}

#[test]
fn report_covers_partitions_and_deploys() {
    let cfg = sample_cfg();
    let layout = layout::compute(&cfg);
    let artifacts = vec![
        Artifact {
            kind: "slot.raw",
            path: PathBuf::from("/x/ingot_0.1.0.slot.raw"),
            size: 322_000_000,
        },
        Artifact {
            kind: "efi",
            path: PathBuf::from("/x/ingot_0.1.0.efi"),
            size: 21_000_000,
        },
        Artifact {
            kind: "esp.raw",
            path: PathBuf::from("/x/ingot_0.1.0.esp.raw"),
            size: 220_000_000,
        },
        Artifact {
            kind: "var.raw",
            path: PathBuf::from("/x/ingot_0.1.0.var.raw"),
            size: 4_800_000,
        },
        Artifact {
            kind: "home.raw",
            path: PathBuf::from("/x/ingot_0.1.0.home.raw"),
            size: 4_800_000,
        },
    ];
    let plan = Plan {
        cfg,
        layout,
        artifacts,
        checksums: vec![None, None, None, None, None],
        disk_bytes: 29 * (1 << 30),
        disk_format: "qcow2".to_string(),
    };
    let report = render_report(&plan);
    for needle in [
        "ingot_0.1.0.slot.raw",
        "slot B stays _empty",
        "f357e520-642b-5b9b-abc7-9e5969de5691",
        "0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
        "59f1269b-a18e-558b-a225-8394c1d1bc0f",
        "501347aa-775a-5736-8da3-2a9977c820ec",
        "9e772d30-44c4-5784-8be1-0832e3c1c4db",
        "tommy (uid 1000, /usr/bin/brush)",
        "sshd.service",
        "hostname=ingot1",
        "29 GiB",
    ] {
        assert!(
            report.contains(needle),
            "report missing {needle:?}\n{report}"
        );
    }
}

#[test]
fn plan_rejects_encrypted_state_in_v1() {
    let cfg = sample_cfg();
    let mut cfg = cfg;
    cfg.enc_var = crate::config::Encryption::Luks2;
    let e = plan(cfg).unwrap_err();
    assert!(e.contains("luks2"), "{e}");
}

#[test]
fn plan_rejects_missing_artifacts() {
    // plan() validates the disk before the artifacts: give it a
    // real (sparse) raw disk so the artifact check is reached.
    let dir = std::env::temp_dir().join(format!("ingot-plan-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let disk = dir.join("target.raw");
    let f = fs::File::create(&disk).unwrap();
    f.set_len(30 * (1 << 30)).unwrap();
    drop(f);

    let mut cfg = sample_cfg();
    cfg.target_disk = disk.to_string_lossy().to_string();
    let e = plan(cfg).unwrap_err();
    assert!(e.contains("artifact not found"), "{e}");
    let _ = fs::remove_dir_all(&dir);
}
