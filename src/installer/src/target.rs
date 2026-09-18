//! Target disk handling.
//!
//! The engine never destroys the original target file in place: a
//! regular-file target (qcow2 or raw) is worked on through a raw
//! working copy in the working directory, and only a successful run
//! replaces the original (atomically, via rename in the same
//! directory). Block-device targets are used directly and are flagged
//! as having no automatic rollback.
//!
//! Mechanics: `qemu-img` for format detection and conversion,
//! `losetup` for the loop device, sysfs (`/sys/block/<n>/*/partuuid`)
//! for partition-device lookup, plain `mount(8)` for the ESP and the
//! state partitions.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::size::human;

/// A prepared target: the original path plus the device the engine
/// operates on.
#[derive(Debug)]
pub struct DiskTarget {
    /// The path the user named in the config.
    pub original: PathBuf,
    /// What `repart`/`dd` operate on (the loop device, the working
    /// raw file, or the block device itself).
    pub disk: PathBuf,
    /// The working raw file (file targets only).
    pub work_file: Option<PathBuf>,
    /// True when `original` is a regular file: on success the working
    /// copy is converted back to the original format and renamed over
    /// it.
    pub working_copy: bool,
    /// True when the original is qcow2 (conversion back is needed).
    pub was_qcow2: bool,
    /// The loop device (`/dev/loopN`), file targets only.
    loop_dev: Option<PathBuf>,
    /// Mount points, in mount order; detached in reverse.
    mounts: Vec<PathBuf>,
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {cmd} ({}); the installer needs it in PATH", e))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} {} failed (exit {}): {}",
            args.join(" "),
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// True when `path` names a block device (has a /sys/block entry).
pub(crate) fn is_block_device(path: &Path) -> bool {
    match path.file_name() {
        Some(name) => Path::new("/sys/block").join(name).is_dir(),
        None => false,
    }
}

/// The size in bytes of the disk a file target represents: the
/// virtual size for qcow2, the file size for raw. The working copy
/// is exactly this big.
fn file_disk_size(path: &Path, fmt: &str) -> Result<u64, String> {
    if fmt == "qcow2" {
        qemu_img_virtual_size(path)
    } else {
        fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| format!("target disk not accessible: {}: {e}", path.display()))
    }
}

/// The virtual size in bytes of a qcow2 disk, from `qemu-img info`
/// (the working copy is exactly this big).
pub(crate) fn qemu_img_virtual_size(path: &Path) -> Result<u64, String> {
    let v: serde_json::Value = qemu_img_info(path)?
        .parse()
        .map_err(|_| "qemu-img info: bad JSON".to_string())?;
    v.get("virtual-size")
        .and_then(|x| x.as_u64())
        .ok_or("qemu-img info: no virtual-size".to_string())
}

/// The working copy occupies the whole disk inside the work volume:
/// verify the volume can hold it before any byte is written (a tmpfs
/// working directory can be smaller than the disk). `df` reports the
/// free space of the volume `work` lives on, in bytes.
fn work_capacity(work: &Path, needed: u64) -> Result<(), String> {
    let out = run("df", &["-P", "-B", "1", &work.to_string_lossy()])
        .map_err(|e| format!("cannot probe working directory capacity: {e}"))?;
    let avail = out
        .lines()
        .last()
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|n| n.parse::<u128>().ok())
        .ok_or("cannot determine free space in the working directory (bad df output)")?;
    if avail < u128::from(needed) {
        return Err(format!(
            "working directory {} has {} available; the working copy of this {} disk needs {} free (pass --work on a larger volume)",
            work.display(),
            human(u64::try_from(avail).unwrap_or(u64::MAX)),
            human(needed),
            human(needed)
        ));
    }
    Ok(())
}

/// Reads the PARTUUID of a block device with `blkid -p` (direct
/// probe, bypassing any cache). Returns None when the device has no
/// partition table metadata.
///
/// In probe mode blkid reports the GPT partition GUID as
/// `PART_ENTRY_UUID` (the `PARTUUID` token is the cache-mode spelling).
fn blkid_partuuid(dev: &str) -> Option<String> {
    let out = Command::new("blkid").args(["-p", dev]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Line shape: /dev/loop8p1: PART_ENTRY_SCHEME="gpt" ...
    //             PART_ENTRY_UUID="..." ...
    let line = text.lines().next()?;
    for key in ["PART_ENTRY_UUID=\"", "PARTUUID=\""] {
        if let Some(field) = line.split_whitespace().find(|f| f.starts_with(key)) {
            return Some(
                field
                    .trim_start_matches(key)
                    .trim_end_matches('"')
                    .to_string(),
            );
        }
    }
    None
}

impl DiskTarget {
    /// Detects the target kind and prepares the working file.
    pub fn prepare(original: &Path, work: &Path) -> Result<Self, String> {
        fs::create_dir_all(work)
            .map_err(|e| format!("cannot create working directory {}: {e}", work.display()))?;

        let meta = fs::metadata(original)
            .map_err(|e| format!("target disk not accessible: {}: {e}", original.display()))?;
        if is_block_device(original) {
            // Used directly: no working copy, no automatic rollback.
            return Ok(DiskTarget {
                original: original.to_path_buf(),
                disk: original.to_path_buf(),
                work_file: None,
                working_copy: false,
                was_qcow2: false,
                loop_dev: None,
                mounts: Vec::new(),
            });
        }
        if !meta.is_file() {
            return Err(format!(
                "target disk {} is neither a file nor a block device",
                original.display()
            ));
        }

        let fmt = qemu_img_format(original)?;
        // The working copy is the whole disk: the work volume must
        // hold it before the first write. A tmpfs working directory
        // can be smaller than the disk - failing here is a
        // diagnostic; failing mid-write is an EIO.
        work_capacity(work, file_disk_size(original, &fmt)?)?;
        let work_file = work.join("target.raw");
        if fmt == "qcow2" {
            run(
                "qemu-img",
                &[
                    "convert",
                    "-f",
                    "qcow2",
                    "-O",
                    "raw",
                    &original.to_string_lossy(),
                    &work_file.to_string_lossy(),
                ],
            )
            .map_err(|e| format!("qcow2 -> raw conversion failed: {e}"))?;
            Ok(DiskTarget {
                original: original.to_path_buf(),
                disk: work_file.clone(),
                work_file: Some(work_file),
                working_copy: true,
                was_qcow2: true,
                loop_dev: None,
                mounts: Vec::new(),
            })
        } else if fmt == "raw" {
            let _ = fs::remove_file(&work_file);
            fs::copy(original, &work_file).map_err(|e| format!("cannot copy raw target: {e}"))?;
            Ok(DiskTarget {
                original: original.to_path_buf(),
                disk: work_file.clone(),
                work_file: Some(work_file),
                working_copy: true,
                was_qcow2: false,
                loop_dev: None,
                mounts: Vec::new(),
            })
        } else {
            Err(format!(
                "unsupported target format {fmt:?} (expected qcow2 or raw)"
            ))
        }
    }

    /// The device repart/dd/partprobe operate on: the loop device
    /// after attach_loop, or the block device itself.
    pub fn active_device(&self) -> &Path {
        self.loop_dev.as_deref().unwrap_or(&self.disk)
    }

    /// Attaches the working file to a free loop device.
    pub fn attach_loop(&mut self) -> Result<(), String> {
        let Some(f) = &self.work_file else {
            return Ok(());
        };
        // -P: partscan, so the kernel picks up the GPT repart wrote
        // before the attach.
        let dev = run("losetup", &["-f", "-P", "--show", &f.to_string_lossy()])
            .map_err(|e| format!("losetup failed: {e}"))?;
        self.loop_dev = Some(PathBuf::from(&dev));
        Ok(())
    }

    /// The sysfs block name of the device under `disk` (`loop3` for
    /// `/dev/loop3`, the device basename for block devices).
    fn sysfs_name(&self) -> String {
        self.active_device()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Finds the partition device whose PARTUUID matches `uuid`,
    /// polling for up to `timeout` seconds.
    ///
    /// PARTUUID is read with `blkid -p` (direct probe, no cache): the
    /// kernel does not expose the `partuuid` sysfs attribute for
    /// loop-device partitions, so blkid is the only source that works
    /// for both loop and block-device targets.
    pub fn partition_by_uuid(&self, uuid: &str, timeout_secs: u64) -> Result<PathBuf, String> {
        let name = self.sysfs_name();
        let base = format!("/sys/block/{name}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            let entries = match fs::read_dir(&base) {
                Ok(e) => e,
                Err(_) => {
                    return Err(format!(
                        "device not in sysfs: {base} (disk {} not attached?)",
                        self.disk.display()
                    ));
                }
            };
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let Some(rest) = n.strip_prefix(&name) else {
                    continue;
                };
                let rest = rest.strip_prefix('p').unwrap_or(rest);
                if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let dev = format!("/dev/{n}");
                if blkid_partuuid(&dev).as_deref() == Some(uuid) {
                    return Ok(PathBuf::from(dev));
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(format!(
                    "partition with PARTUUID {uuid} never appeared under {base}"
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// Mounts `dev` (fstype `fs`) at a fresh directory under `work`.
    /// `opts` is appended to `-o` (empty for defaults).
    pub fn mount(
        &mut self,
        dev: &Path,
        fs: &str,
        work: &Path,
        opts: &str,
    ) -> Result<PathBuf, String> {
        let dir = work.join(format!(
            "mount-{}",
            dev.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "dev".into())
        ));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let d = dev.to_string_lossy().to_string();
        let m = dir.to_string_lossy().to_string();
        if opts.is_empty() {
            run("mount", &["-t", fs, &d, &m])?;
        } else {
            run("mount", &["-t", fs, "-o", opts, &d, &m])?;
        }
        self.mounts.push(dir.clone());
        Ok(dir)
    }

    /// Unmounts everything (reverse order) and detaches the loop
    /// device. Best-effort: the first failure is reported, the rest is
    /// still attempted.
    pub fn detach_all(&mut self) -> Result<(), String> {
        let mut first_err: Option<String> = None;
        for dir in self.mounts.iter().rev() {
            let d = dir.to_string_lossy().to_string();
            if let Err(e) = run("umount", &[&d]) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        self.mounts.clear();
        if let Some(loop_dev) = &self.loop_dev {
            let d = loop_dev.to_string_lossy().to_string();
            if let Err(e) = run("losetup", &["-d", &d]) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        self.loop_dev = None;
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Finishes a successful run: converts the working file back to
    /// the original format and renames it over the original (file
    /// targets only).
    pub fn commit(&self) -> Result<(), String> {
        let Some(work_file) = &self.work_file else {
            return Ok(());
        };
        if !self.was_qcow2 {
            // raw -> raw: plain copy, then atomic rename.
            let tmp = self.original.with_extension("ingot-install.tmp");
            fs::copy(work_file, &tmp).map_err(|e| format!("cannot write installed image: {e}"))?;
            fs::rename(&tmp, &self.original)
                .map_err(|e| format!("cannot replace {}: {e}", self.original.display()))?;
            return Ok(());
        }
        let tmp = self.original.with_extension("qcow2.tmp");
        let w = work_file.to_string_lossy().to_string();
        let t = tmp.to_string_lossy().to_string();
        let o = self.original.to_string_lossy().to_string();
        run("qemu-img", &["convert", "-f", "raw", "-O", "qcow2", &w, &t])
            .map_err(|e| format!("raw -> qcow2 conversion failed: {e}"))?;
        fs::rename(&tmp, &self.original).map_err(|e| format!("cannot replace {}: {e}", o))?;
        Ok(())
    }

    /// Removes the working file (the log and definitions are kept in
    /// the working directory for post-mortems).
    pub fn cleanup_work_file(&self) {
        if let Some(f) = &self.work_file {
            let _ = fs::remove_file(f);
        }
    }
}

/// Runs `qemu-img info --output json` on a file and returns the
/// JSON text.
pub fn qemu_img_info(path: &Path) -> Result<String, String> {
    run(
        "qemu-img",
        &["info", "--output", "json", &path.to_string_lossy()],
    )
    .map_err(|e| format!("qemu-img info failed: {e}"))
}

/// Queries `qemu-img info` for a file's format.
pub fn qemu_img_format(path: &Path) -> Result<String, String> {
    let v: serde_json::Value = qemu_img_info(path)?
        .parse()
        .map_err(|_| "qemu-img info: bad JSON".to_string())?;
    v.get("format")
        .and_then(|x| x.as_str())
        .map(String::from)
        .ok_or("qemu-img info: no format".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_capacity_ok_and_too_small() {
        let dir = std::env::temp_dir().join("ingot-target-cap-test");
        fs::create_dir_all(&dir).unwrap();
        assert!(work_capacity(&dir, 1).is_ok());
        let err = work_capacity(&dir, u64::MAX).unwrap_err();
        assert!(err.contains("working directory"), "{err}");
        assert!(err.contains("--work"), "{err}");
        let _ = fs::remove_dir(&dir);
    }
}
