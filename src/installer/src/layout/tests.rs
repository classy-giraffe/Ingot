
use super::*;

fn cfg() -> Config {
    config::parse(
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
[ssh]
authorized_keys = []
[services]
enabled = []
"#,
    )
    .unwrap()
}

#[test]
fn layout_has_fixed_uuids_and_order() {
    let layout = compute(&cfg());
    let uuids: Vec<&str> = layout.partitions.iter().map(|p| p.part_uuid).collect();
    assert_eq!(
        uuids,
        vec![ESP_UUID, SLOT_A_UUID, SLOT_B_UUID, VAR_UUID, HOME_UUID]
    );
    let labels: Vec<&str> = layout.partitions.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, vec!["esp", "ingot_0.1.0", "_empty", "var", "home"]);
    assert_eq!(
        layout
            .partitions
            .iter()
            .map(|p| p.gpt_type)
            .collect::<Vec<_>>(),
        vec!["esp", "usr", "usr", "var", "home"]
    );
    assert_eq!(
        layout
            .partitions
            .iter()
            .map(|p| p.fs.as_str())
            .collect::<Vec<_>>(),
        vec!["vfat", "erofs", "unformatted", "btrfs", "btrfs"]
    );
}

#[test]
fn offsets_are_cumulative_from_one_mib() {
    let layout = compute(&cfg());
    let sizes: Vec<u64> = layout.partitions.iter().map(|p| p.size).collect();
    let mut offset = FIRST_OFFSET;
    for p in &layout.partitions {
        assert_eq!(p.offset, offset);
        offset += p.size;
    }
    assert_eq!(layout.used_bytes, offset);
    assert_eq!(sizes, vec![1 << 30, 8 << 30, 8 << 30, 4 << 30, 8 << 30]);
}

#[test]
fn required_disk_is_used_plus_headroom() {
    let layout = compute(&cfg());
    assert_eq!(layout.required_disk_bytes, layout.used_bytes + size::MIB);
    // exact fit passes, one byte short fails
    assert!(check_disk(&layout, layout.required_disk_bytes).is_ok());
    assert!(check_disk(&layout, layout.required_disk_bytes - 1).is_err());
}

#[test]
fn versioned_slot_label_tracks_version() {
    let doc = r#"
schema = 1
[target]
disk = "/dev/vda"
[source]
base = "dist"
version = "2.10.3"
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
home = "ext4"
[encryption]
var = "none"
home = "none"
[[users]]
name = "tommy"
[ssh]
authorized_keys = []
[services]
enabled = []
"#;
    let layout = compute(&config::parse(doc).unwrap());
    assert_eq!(layout.partitions[1].label, "ingot_2.10.3");
    assert_eq!(layout.partitions[1].label, slot_a_label(&layout.version));
    assert_eq!(layout.partitions[4].fs, "ext4");
}

#[test]
fn repart_defs_render_baseline_exact() {
    let layout = compute(&cfg());
    let defs = repart_defs(&layout);
    assert_eq!(defs.len(), 5);
    let lines = |i: usize| defs[i].1.lines().collect::<Vec<_>>();
    // The defs are baseline-pinned (image/repart-baseline/): the
    // exact rendered content is the contract. In particular the
    // Label line must stand alone - a glued "Label=xFormat=y"
    // line corrupts the GPT label repart writes.
    assert_eq!(
        lines(0),
        vec![
            "[Partition]",
            "Type=esp",
            &format!("UUID={ESP_UUID}"),
            "Label=esp",
            "Format=vfat",
            "SizeMinBytes=1073741824",
            "SizeMaxBytes=1073741824",
        ],
        "{}",
        defs[0].1
    );
    assert_eq!(
        lines(1),
        vec![
            "[Partition]",
            "Type=usr",
            &format!("UUID={SLOT_A_UUID}"),
            "Label=ingot_0.1.0",
            "Format=erofs",
            "Compression=zstd",
            "SizeMinBytes=8589934592",
            "SizeMaxBytes=8589934592",
        ],
        "{}",
        defs[1].1
    );
    assert_eq!(
        lines(2),
        vec![
            "[Partition]",
            "Type=usr",
            &format!("UUID={SLOT_B_UUID}"),
            "Label=_empty",
            "SizeMinBytes=8589934592",
            "SizeMaxBytes=8589934592",
        ],
        "{}",
        defs[2].1
    );
    assert_eq!(
        lines(3),
        vec![
            "[Partition]",
            "Type=var",
            &format!("UUID={VAR_UUID}"),
            "Label=var",
            "Format=btrfs",
            "SizeMinBytes=4294967296",
            "SizeMaxBytes=4294967296",
        ],
        "{}",
        defs[3].1
    );
    assert_eq!(
        lines(4),
        vec![
            "[Partition]",
            "Type=home",
            &format!("UUID={HOME_UUID}"),
            "Label=home",
            "Format=btrfs",
            "SizeMinBytes=8589934592",
            "SizeMaxBytes=8589934592",
        ],
        "{}",
        defs[4].1
    );
    assert_eq!(defs[0].0, "1-esp.conf");
    assert_eq!(defs[1].0, "2-ingot_0.1.0.conf");
    assert_eq!(defs[2].0, "3-_empty.conf");
    assert_eq!(defs[3].0, "4-var.conf");
    assert_eq!(defs[4].0, "5-home.conf");
}
