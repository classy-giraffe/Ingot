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
pub const MIB: u64 = 1 << 20;

/// First partition offset (after the protective MBR and align gap).
pub const FIRST_OFFSET: u64 = MIB;

// Fixed PARTUUIDs (image/repart-baseline). MUST NOT drift from the
// image build; the UKI hardcodes them.
pub const ESP_UUID: &str = "f357e520-642b-559b-abc7-9e5969e56911";
pub const SLOT_A_UUID: &str = "0066bfe5-4f71-52dc-9a16-bb10191a1ddc";
pub const SLOT_B_UUID: &str = "5016269b-a18e-556b-a225-83941cd1bcaf";
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
    /// Filesystem the partition is formatted as (`Format=`).
    pub fs: &'static str,
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

    /// Filesystem for this role given the config's choices.
    pub fn fs(self, cfg: &Config) -> &'static str {
        match self {
            Role::Esp => "fat",
            Role::SlotA | Role::SlotB => match cfg.fs_slot {
                config::SlotFs::Eros => "erofs",
            },
            Role::Var => match cfg.fs_var {
                StateFs::Btrfs => "btrfs",
                StateFs::Ext4 => "ext4",
            },
            Role::Home => match cfg.fs_home {
                StateFs::Btrfs => "btrfs",
                StateFs::Ext4 => "ext4",
            },
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
            fs: role.fs(cfg),
            size,
            offset,
        });
        offset += size;
    }
    let used_bytes = offset;
    // GPT tail metadata needs a little room; require 1 MiB headroom.
    let required_disk_bytes = used_bytes + MIB;
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
            let conf = format!(
                "[Partition]\nType={gpt}\nUUID={uuid}\nLabel={label}\nFormat={fs}\nSizeMinBytes={size}\nSizeMaxBytes={size}\n",
                gpt = p.gpt_type,
                uuid = p.part_uuid,
                label = p.label,
                fs = p.fs,
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
            layout.partitions.iter().map(|p| p.gpt_type).collect::<Vec<_>>(),
            vec!["esp", "usr", "usr", "var", "home"]
        );
        assert_eq!(
            layout.partitions.iter().map(|p| p.fs).collect::<Vec<_>>(),
            vec!["fat", "erofs", "erofs", "btrfs", "btrfs"]
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
        assert_eq!(
            sizes,
            vec![1 << 30, 8 << 30, 8 << 30, 4 << 30, 8 << 30]
        );
    }

    #[test]
    fn required_disk_is_used_plus_headroom() {
        let layout = compute(&cfg());
        assert_eq!(
            layout.required_disk_bytes,
            layout.used_bytes + MIB
        );
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
    fn repart_defs_cover_all_partitions() {
        let layout = compute(&cfg());
        let defs = repart_defs(&layout);
        assert_eq!(defs.len(), 5);
        let esp = &defs[0].1;
        assert!(esp.contains("Type=esp"), "{esp}");
        assert!(esp.contains(&format!("UUID={ESP_UUID}")), "{esp}");
        assert!(esp.contains("Label=esp"), "{esp}");
        assert!(esp.contains("Format=fat"), "{esp}");
        assert!(esp.contains("SizeMinBytes=1073741824"), "{esp}");
        assert!(esp.contains("SizeMaxBytes=1073741824"), "{esp}");
        let slot_a = &defs[1].1;
        assert!(slot_a.contains("Type=usr"), "{slot_a}");
        assert!(slot_a.contains("Label=ingot_0.1.0"), "{slot_a}");
        assert!(slot_a.contains("Format=erofs"), "{slot_a}");
        let slot_b = &defs[2].1;
        assert!(slot_b.contains("Label=_empty"), "{slot_b}");
        assert_eq!(defs[0].0, "1-esp.conf");
        assert_eq!(defs[1].0, "2-ingot_0.1.0.conf");
        assert_eq!(defs[2].0, "3-_empty.conf");
        assert_eq!(defs[3].0, "4-var.conf");
        assert_eq!(defs[4].0, "5-home.conf");
    }
}
