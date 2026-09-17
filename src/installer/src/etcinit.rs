//! /var/lib/etc initialization (spec 9.3, 11.4.3-5, .9, .10, .11).
//!
//! The deployed /var partition starts empty: the factory defaults
//! live in the slot at `share/factory/etc` (spec 9.3), and first
//! boot initializes `/etc` from them if empty. The installer does
//! that non-interactively - it seeds `/var/lib/etc` from the slot's
//! factory defaults (skipped when the /var partition already carries
//! state) and then overlays the install config onto it:
//!
//! - `hostname`, `localtime` (symlink into the slot's tzdata),
//!   `locale.conf`, `vconsole.conf`
//! - initial users appended to `passwd`/`group`/`shadow` (locked
//!   password `!`, no password auth at first boot) with home
//!   directories on the /home partition
//! - SSH authorized keys into each user's home
//! - service enablement via `multi-user.target.wants` symlinks,
//!   checked against the units the installed release actually ships

use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::log::InstallLog;

/// Appends a line to a file, creating it with 0644 if missing.
fn append_line(path: &Path, line: &str) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o644)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    writeln!(f, "{line}").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Creates the home directory with owner and permissions.
fn make_home(home_root: &Path, user: &str, uid: u32, gid: u32) -> Result<PathBuf, String> {
    let dir = home_root.join(user);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create home for {user}: {e}"))?;
    let meta = fs::metadata(&dir).map_err(|e| e.to_string())?;
    if meta.uid() != uid {
        std::os::unix::fs::chown(&dir, Some(uid), Some(gid))
            .map_err(|e| format!("cannot chown home for {user}: {e}"))?;
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o750))
        .map_err(|e| format!("cannot chmod home for {user}: {e}"))?;
    Ok(dir)
}

/// Recursively copies `src` into `dst` (directories, files, symlinks),
/// preserving permissions. Existing files in `dst` are never
/// overwritten.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src).map_err(|e| e.to_string())?;
        fs::create_dir_all(dst.parent().unwrap_or(dst)).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(dst);
        std::os::unix::fs::symlink(target, dst)
            .map_err(|e| format!("cannot symlink {}: {e}", dst.display()))
    } else if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_tree(entry.path().as_path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::create_dir_all(dst.parent().unwrap_or(dst)).map_err(|e| e.to_string())?;
        if dst.exists() {
            return Ok(());
        }
        fs::copy(src, dst)
            .map_err(|e| format!("cannot copy {} -> {}: {e}", src.display(), dst.display()))?;
        fs::set_permissions(dst, fs::Permissions::from_mode(meta.permissions().mode()))
            .map_err(|e| e.to_string())
    }
}

/// Seeds an empty /var/lib/etc from the slot's factory defaults
/// (spec 9.3: factory defaults in the payload, first boot initializes
/// /etc from them if empty).
fn seed_factory(var: &Path, slot: &Path, log: &InstallLog) -> Result<(), String> {
    let factory = slot.join("share/factory/etc");
    if !factory.is_dir() {
        return Err(format!(
            "factory defaults missing from the installed release ({}): cannot initialize /var/lib/etc",
            factory.display()
        ));
    }
    let etc = var.join("lib/etc");
    copy_tree(&factory, &etc)
        .map_err(|e| format!("cannot seed /var/lib/etc from the factory defaults: {e}"))?;
    log.log_result(
        "etcinit",
        "factory-seeded",
        Some(format!(
            "/var/lib/<redacted> seeded from /usr/share/factory/etc"
        )),
    )
}

/// Initializes /var/lib/etc and the home directories from the
/// factory state plus the install config.
///
/// `var` and `home` are the mounted state partitions; `slot` is the
/// mounted active slot (read-only), used to check timezone data and
/// the units the release ships.
pub fn run(
    cfg: &Config,
    var: &Path,
    home: &Path,
    slot: &Path,
    log: &InstallLog,
) -> Result<(), String> {
    let etc = var.join("lib/etc");
    if !etc.is_dir() {
        seed_factory(var, slot, log)?;
    }

    // --- system identity -------------------------------------------------
    fs::write(etc.join("hostname"), format!("{}\n", cfg.hostname))
        .map_err(|e| format!("cannot write hostname: {e}"))?;
    log.log_result("etcinit", "hostname", Some(cfg.hostname.clone()))?;

    // Verify the zone exists in the installed release, then link the
    // runtime path: the slot mounts at /usr on the installed system,
    // so /usr/share/zoneinfo/<tz> is stable there (linking the
    // install-time mount point would dangle).
    let tz_rel = format!("share/zoneinfo/{}", cfg.timezone);
    if !slot.join(&tz_rel).is_file() {
        return Err(format!(
            "timezone {0:?} not found in the installed release (expected /usr/{1})",
            cfg.timezone, tz_rel
        ));
    }
    let lt = etc.join("localtime");
    let _ = fs::remove_file(&lt);
    std::os::unix::fs::symlink(format!("/usr/{tz_rel}"), &lt)
        .map_err(|e| format!("cannot link localtime: {e}"))?;
    log.log_result("etcinit", "timezone", Some(cfg.timezone.clone()))?;

    fs::write(etc.join("locale.conf"), format!("LANG={}\n", cfg.locale))
        .map_err(|e| format!("cannot write locale.conf: {e}"))?;
    fs::write(
        etc.join("vconsole.conf"),
        format!("KEYMAP={}\n", cfg.keymap),
    )
    .map_err(|e| format!("cannot write vconsole.conf: {e}"))?;
    log.log_result(
        "etcinit",
        "locale-keymap",
        Some(format!("{} / {}", cfg.locale, cfg.keymap)),
    )?;

    // --- users ------------------------------------------------------------
    // uids start at 1000 (spec: the first user is the administrator).
    let mut uid = 1000u32;
    let mut gid = 1000u32;
    for u in &cfg.users {
        if !crate::config::is_user_name(&u.name) {
            return Err(format!("user name {:?} is invalid", u.name));
        }
        let shell = u
            .shell
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_SHELL.to_string());
        // The slot partition root is the /usr tree: a runtime
        // /usr/bin/shell is bin/shell inside the slot.
        let slot_shell = shell
            .trim_start_matches('/')
            .strip_prefix("usr/")
            .unwrap_or_else(|| shell.trim_start_matches('/'));
        if !slot.join(slot_shell).is_file() {
            return Err(format!(
                "shell {shell} does not exist in the installed release"
            ));
        }
        append_line(
            &etc.join("passwd"),
            &format!(
                "{name}:x:{uid}:{gid}:{name}:/home/{name}:{shell}",
                name = u.name
            ),
        )?;
        append_line(&etc.join("group"), &format!("{}:x:{gid}:", u.name))?;
        append_line(
            &etc.join("shadow"),
            &format!("{}:!:19000:0:99999:7:::", u.name),
        )?;
        let homedir = make_home(home, &u.name, uid, gid)?;

        // SSH authorized keys: every key to every initial user.
        if !cfg.ssh_keys.is_empty() {
            let sshdir = homedir.join(".ssh");
            fs::create_dir_all(&sshdir).map_err(|e| e.to_string())?;
            fs::set_permissions(&sshdir, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            let mut keys = String::new();
            for k in &cfg.ssh_keys {
                keys.push_str(k);
                if !k.ends_with('\n') {
                    keys.push('\n');
                }
            }
            fs::write(sshdir.join("authorized_keys"), keys)
                .map_err(|e| format!("cannot write authorized_keys for {}: {e}", u.name))?;
            std::os::unix::fs::chown(&sshdir, Some(uid), Some(gid))
                .map_err(|e| format!("cannot chown .ssh for {}: {e}", u.name))?;
            std::os::unix::fs::chown(sshdir.join("authorized_keys"), Some(uid), Some(gid))
                .map_err(|e| format!("cannot chown authorized_keys for {}: {e}", u.name))?;
            fs::set_permissions(
                sshdir.join("authorized_keys"),
                fs::Permissions::from_mode(0o600),
            )
            .map_err(|e| e.to_string())?;
        }
        log.log_result(
            "etcinit",
            "user",
            Some(format!("{} uid={uid} shell={shell}", u.name)),
        )?;
        uid += 1;
        gid += 1;
    }

    // The account files: shadow must not be world-readable (the
    // append helper creates files with 0644).
    for (file, mode) in [("shadow", 0o600u32), ("passwd", 0o644), ("group", 0o644)] {
        let p = etc.join(file);
        if p.exists() {
            fs::set_permissions(&p, fs::Permissions::from_mode(mode))
                .map_err(|e| format!("cannot set permissions on /var/lib/etc/{file}: {e}"))?;
        }
    }

    // --- services ---------------------------------------------------------
    let wants = etc.join("systemd/system/multi-user.target.wants");
    for unit in &cfg.services {
        if !crate::config::is_unit_name(unit) {
            return Err(format!("service {unit:?} is not a valid systemd unit name"));
        }
        let shipped = slot.join(format!("lib/systemd/system/{unit}"));
        if !shipped.is_file() {
            log.log_result(
                "etcinit",
                "service-skip",
                Some(format!(
                    "{unit}: not shipped by the installed release, not enabled"
                )),
            )?;
            continue;
        }
        fs::create_dir_all(&wants).map_err(|e| e.to_string())?;
        let link = wants.join(unit);
        let target = format!("/usr/lib/systemd/system/{unit}");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(target, &link)
            .map_err(|e| format!("cannot enable {unit}: {e}"))?;
        log.log_result("etcinit", "service-enabled", Some(unit.clone()))?;
    }

    Ok(())
}

