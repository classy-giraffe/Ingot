//! Disk layout computation: the partition plan an install config
//! expands into, with the fixed PARTUUIDs baked into the T1 UKI.
//!
//! The fixed UUIDs come from the repart definitions that build the
//! image (`image/repart-baseline/`); the UKI's kernel command line
//! carries the slot and state PARTUUIDs, so an install must reuse
//! exactly these values or the deployed image cannot boot.
//!
//! Offsets are cumulative from 1 MiB (GPT protective header + first
//! align gap), in the same order the repart definitions declare:
//! esp, slot A, slot B, var, home.

use crate::{config, size, version};
use config::{Config, StateFs};
use size::ByteSize;
use version::Version;

/// One mebibyte.

/// First partition offset (after the protective MBR and align gap).
pub const FIRST_OFFSET: u64 = size::MIB;

// Fixed PARTUUIDs (image/repart-baseline). MUST NOT drift from the
// image build; the UKI hardcodes them.
pub const ESP_UUID: &str = "f357e520-642b-5b9b-abc7-9e5969de5691";
pub const SLOT_A_UUID: &str = "0066bfe5-47f1-52dc-9a16-1bb10191a1dc";
pub const SLOT_B_UUID: &str = "59f1269b-a18e-558b-a225-8394c1d1bc0f";
pub const VAR_UUID: &str = "501347aa-775a-5736-8da3-2a9977c820ec";
pub const HOME_UUID: &str = "9e772d30-44c4-5784-8be1-0832e3c1c4db";

/// Partition role in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Esp,
    SlotA,
    SlotB,
    Var,
    Home,
}

/// One partition of the plan.
#[derive(Debug, Clone)]
pub struct PartitionPlan {
    /// GPT partition number (1-based).
    pub index: u32,
    pub role: Role,
    /// GPT partition name (the label the kernel/loader see).
    pub label: String,
    /// GPT type GUID shorthand for the plan report.
    pub gpt_type: &'static str,
    /// Fixed PARTUUID (no dashes stripped; the full canonical form).
    pub part_uuid: &'static str,
    /// Filesystem the partition is formatted as (`Format=`);
    /// "unformatted" for the free slot B.
    pub fs: String,
    /// Partition size in bytes.
    pub size: u64,
    /// Byte offset of the partition start within the disk.
    pub offset: u64,
}

/// The full partition plan for one install.
#[derive(Debug, Clone)]
pub struct Layout {
    pub version: Version,
    pub partitions: Vec<PartitionPlan>,
    /// Bytes actually used: first offset + all partition sizes.
    pub used_bytes: u64,
    /// Minimum target-disk size the plan fits in.
    pub required_disk_bytes: u64,
}

impl Role {
    /// GPT type shorthand for reports.
    pub fn gpt_type(self) -> &'static str {
        match self {
            Role::Esp => "esp",
            Role::SlotA | Role::SlotB => "usr",
            Role::Var => "var",
            Role::Home => "home",
        }
    }

    /// Fixed PARTUUID.
    pub fn part_uuid(self) -> &'static str {
        match self {
            Role::Esp => ESP_UUID,
            Role::SlotA => SLOT_A_UUID,
            Role::SlotB => SLOT_B_UUID,
            Role::Var => VAR_UUID,
            Role::Home => HOME_UUID,
        }
    }

    /// Filesystem for this role given the config's choices; the free
    /// slot B is left unformatted (an update fills it later).
    pub fn fs(self, cfg: &Config) -> Option<&'static str> {
        match self {
            Role::Esp => Some("vfat"),
            Role::SlotA => Some(match cfg.fs_slot {
                config::SlotFs::Eros => "erofs",
            }),
            Role::SlotB => None,
            Role::Var => Some(match cfg.fs_var {
                StateFs::Btrfs => "btrfs",
                StateFs::Ext4 => "ext4",
            }),
            Role::Home => Some(match cfg.fs_home {
                StateFs::Btrfs => "btrfs",
                StateFs::Ext4 => "ext4",
            }),
        }
    }

    /// Size for this role from the config.
    pub fn size(self, cfg: &Config) -> ByteSize {
        match self {
            Role::Esp => cfg.esp,
            Role::SlotA => cfg.slot_a,
            Role::SlotB => cfg.slot_b,
            Role::Var => cfg.var,
            Role::Home => cfg.home,
        }
    }
}

/// Computes the partition plan from a validated config.
pub fn compute(cfg: &Config) -> Layout {
    let roles = [Role::Esp, Role::SlotA, Role::SlotB, Role::Var, Role::Home];
    let mut offset = FIRST_OFFSET;
    let mut partitions = Vec::with_capacity(roles.len());
    let mut index = 0u32;
    for role in roles {
        index += 1;
        let size = role.size(cfg).0;
        let label = match role {
            Role::Esp => "esp".to_string(),
            Role::SlotA => slot_a_label(&cfg.version),
            Role::SlotB => "_empty".to_string(),
            Role::Var => "var".to_string(),
            Role::Home => "home".to_string(),
        };
        partitions.push(PartitionPlan {
            index,
            role,
            label,
            gpt_type: role.gpt_type(),
            part_uuid: role.part_uuid(),
            fs: role.fs(cfg).unwrap_or("unformatted").to_string(),
            size,
            offset,
        });
        offset += size;
    }
    let used_bytes = offset;
    // GPT tail metadata needs a little room; require 1 MiB headroom.
    let required_disk_bytes = used_bytes + size::MIB;
    Layout {
        version: cfg.version.clone(),
        partitions,
        used_bytes,
        required_disk_bytes,
    }
}

/// The installed-version label for slot A (the versioned GPT label).
fn slot_a_label(version: &Version) -> String {
    format!("ingot_{version}")
}

/// Checks the plan fits the target disk.
pub fn check_disk(layout: &Layout, disk_bytes: u64) -> Result<(), String> {
    if disk_bytes >= layout.required_disk_bytes {
        Ok(())
    } else {
        Err(format!(
            "target disk is {} bytes; the layout needs {} bytes ({} GiB minimum)",
            disk_bytes,
            layout.required_disk_bytes,
            layout.required_disk_bytes / (1 << 30)
        ))
    }
}

/// Renders the systemd-repart definitions (one file per partition,
/// in partition order) for this layout. The engine writes these to a
/// definitions directory and runs `systemd-repart --definitions=` on
/// the target. Sizes are exact (SizeMinBytes = SizeMaxBytes): the
/// install places whole-image slot artifacts and the state partitions
/// must match the UKI-era geometry assumptions.
pub fn repart_defs(layout: &Layout) -> Vec<(String, String)> {
    layout
        .partitions
        .iter()
        .map(|p| {
            let name = format!("{}-{}.conf", p.index, p.label);
        let format_line = if p.fs == "unformatted" {
            String::new()
        } else if p.fs == "erofs" {
            // Match the image build baseline (image/repart-baseline/
            // 20-slot-a.conf): whole-image deployment overwrites the
            // formatted filesystem, but the def stays baseline-pinned.
            "Format=erofs\nCompression=zstd\n".to_string()
        } else {
            format!("Format={}\n", p.fs)
        };
        let conf = format!(
            "[Partition]\nType={gpt}\nUUID={uuid}\nLabel={label}\n{fmt}SizeMinBytes={size}\nSizeMaxBytes={size}\n",
            gpt = p.gpt_type,
            uuid = p.part_uuid,
            label = p.label,
            fmt = format_line,
            size = p.size,
        );
        (name, conf)
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
}
