//! Host and disk environment checks before installation phases execute.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The subprocesses the engine drives (11.5 phase 1: availability
/// checked before the first write). The mkfs binaries are added per
/// layout in `run`; `bootctl` is absent (best-effort at runtime).
pub const TOOLS: [&str; 8] = [
    "systemd-repart",
    "dd",
    "losetup",
    "blkid",
    "mount",
    "umount",
    "sync",
    "df",
];

/// Names from `TOOLS` plus `extra` with no executable found in
/// `PATH`. A plain PATH scan: no subprocess, unit-testable.
pub fn missing_tools(path_var: &str, extra: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    for name in TOOLS
        .iter()
        .copied()
        .chain(extra.iter().map(String::as_str))
    {
        let found = path_var.split(':').any(|dir| {
            let Ok(meta) = fs::metadata(Path::new(dir).join(name)) else {
                return false;
            };
            meta.is_file() && meta.permissions().mode() & 0o111 != 0
        });
        if !found {
            missing.push(name.to_string());
        }
    }
    missing
}

/// True when `path` is a mount point.
pub fn is_mounted(path: &Path) -> bool {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return false;
    };
    let want = path.to_string_lossy().replace(' ', "\\040");
    for line in mounts.lines() {
        let mut f = line.splitn(3, ' ');
        f.next();
        if f.next() == Some(&want) {
            return true;
        }
    }
    false
}
