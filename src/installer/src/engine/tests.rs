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
        source: PlanSource::Artifacts {
            artifacts,
            checksums: vec![None, None, None, None, None],
        },
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

/// A minimal synthetic PE (x86_64, zero optional header) with the
/// sections validate_uki requires: the same fixture shape as the
/// ukify tests.
fn pe_uki() -> Vec<u8> {
    const CMDLINE: &[u8] =
        b"usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec";
    const OSREL: &[u8] = b"NAME=Ingot\nVERSION_ID=\"0.1.0\"\nID=ingot\n";
    let sections = [(".cmdline", CMDLINE), (".osrel", OSREL)];
    let header_end = 0x58 + 40 * sections.len();
    let mut out = vec![0u8; header_end];
    out[0] = b'M';
    out[1] = b'Z';
    out[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    out[0x40..0x44].copy_from_slice(b"PE\0\0");
    out[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    out[0x46..0x48].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    let mut data_off = header_end;
    for (i, (name, data)) in sections.iter().enumerate() {
        let s = 0x58 + 40 * i;
        let name_bytes: Vec<u8> = name.bytes().take(8).collect();
        out[s..s + name_bytes.len()].copy_from_slice(&name_bytes);
        out[s + 16..s + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
        out[s + 20..s + 24].copy_from_slice(&(data_off as u32).to_le_bytes());
        data_off += data.len();
    }
    for (_, data) in sections.iter() {
        out.extend_from_slice(data);
    }
    out
}

/// A live media fixture: the live payload erofs plus the installed
/// ESP tree (a synthetic UKI with the fixed PARTUUIDs).
fn live_media(dir: &std::path::Path) {
    let erofs = dir.join(crate::source::LIVE_EROFS);
    fs::create_dir_all(erofs.parent().unwrap()).unwrap();
    fs::write(&erofs, b"erofs").unwrap();
    let uki = dir.join("esp/EFI/Linux/ingot_0.1.0.efi");
    fs::create_dir_all(uki.parent().unwrap()).unwrap();
    fs::write(&uki, pe_uki()).unwrap();
    fs::create_dir_all(dir.join("esp/loader")).unwrap();
    fs::write(dir.join("esp/loader/loader.conf"), "timeout 3\n").unwrap();
    fs::create_dir_all(dir.join("esp/EFI/BOOT")).unwrap();
    fs::write(dir.join("esp/EFI/BOOT/BOOTX64.EFI"), b"boot").unwrap();
}

#[test]
fn live_plan_resolves_media_source() {
    let dir = std::env::temp_dir().join(format!("ingot-live-plan-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let media = dir.join("media");
    live_media(&media);
    let disk = dir.join("target.raw");
    let f = fs::File::create(&disk).unwrap();
    f.set_len(30 * (1 << 30)).unwrap();
    drop(f);

    let mut cfg = sample_cfg();
    cfg.source_mode = crate::config::SourceMode::Live;
    cfg.source_base = media.to_string_lossy().to_string();
    cfg.target_disk = disk.to_string_lossy().to_string();
    let plan = plan_with(cfg, Some(crate::version::parse("0.1.0").unwrap()))
        .unwrap();
    match &plan.source {
        PlanSource::Live(live) => {
            assert_eq!(live.erofs, media.join(crate::source::LIVE_EROFS));
            assert_eq!(live.uki, media.join("esp/EFI/Linux/ingot_0.1.0.efi"));
        }
        other => panic!("expected a live plan source, got {other:?}"),
    }
    assert_eq!(plan.layout.version, crate::version::parse("0.1.0").unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn live_plan_names_missing_media_files() {
    let dir = std::env::temp_dir().join(format!("ingot-live-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let media = dir.join("media");
    live_media(&media);
    fs::remove_file(media.join(crate::source::LIVE_EROFS)).unwrap();
    let disk = dir.join("target.raw");
    let f = fs::File::create(&disk).unwrap();
    f.set_len(30 * (1 << 30)).unwrap();
    drop(f);

    let mut cfg = sample_cfg();
    cfg.source_mode = crate::config::SourceMode::Live;
    cfg.source_base = media.to_string_lossy().to_string();
    cfg.target_disk = disk.to_string_lossy().to_string();
    let e = plan_with(cfg, Some(crate::version::parse("0.1.0").unwrap())).unwrap_err();
    assert!(e.contains(crate::source::LIVE_EROFS), "{e}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn live_report_covers_media_deploy() {
    let dir = std::env::temp_dir().join(format!("ingot-live-report-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let media = dir.join("media");
    live_media(&media);
    let disk = dir.join("target.raw");
    let f = fs::File::create(&disk).unwrap();
    f.set_len(30 * (1 << 30)).unwrap();
    drop(f);

    let mut cfg = sample_cfg();
    cfg.source_mode = crate::config::SourceMode::Live;
    cfg.source_base = media.to_string_lossy().to_string();
    cfg.target_disk = disk.to_string_lossy().to_string();
    let plan = plan_with(cfg, Some(crate::version::parse("0.1.0").unwrap()))
        .unwrap();
    let report = render_report(&plan);
    for needle in [
        "live source: the running release on the live ISO",
        "LiveOS/rootfs.erofs",
        "ESP tree: systemd-boot + UKI + loader",
        "Live source (sha256 of the media's files)",
        "format: raw",
    ] {
        assert!(report.contains(needle), "report missing {needle:?}\n{report}");
    }
    let _ = fs::remove_dir_all(&dir);
}
