//! /var/lib/etc initialization (spec 9.3, 11.4.3-5, .9, .10, .11).
//!
//! The /var artifact already carries the factory defaults
//! (`/var/lib/etc` = the factory `/etc` skeleton, spec 9.3). This
//! phase overlays the install config onto it:
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

/// Validates a user name before it is written into passwd/group.
fn check_user_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 32 {
        return Err(format!("user name {name:?} must be 1-32 characters"));
    }
    if name.starts_with('-') || name.starts_with('.') {
        return Err(format!("user name {name:?} must not start with '-' or '.'"));
    }
    if name
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
    {
        return Err(format!("user name {name:?} has invalid characters"));
    }
    if [
        "root", "nobody", "daemon", "bin", "sys", "adm", "lp", "mail", "news", "uucp", "operator",
        "games", "ftp", "nuux", "nobody4",
    ]
    .contains(&name.to_ascii_lowercase().as_str())
    {
        return Err(format!("user name {name:?} collides with a system account"));
    }
    Ok(())
}

/// Validates a systemd unit name.
fn check_unit_name(unit: &str) -> Result<(), String> {
    let valid = !unit.is_empty()
        && unit.len() <= 255
        && unit.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | ':' | '+' | '~' | '-')
        });
    if !valid || !unit.contains('.') {
        return Err(format!("service {unit:?} is not a valid systemd unit name"));
    }
    Ok(())
}

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
        return Err(format!(
            "/var/lib/etc is missing on the deployed /var partition ({}): the factory state is corrupt",
            var.display()
        ));
    }

    // --- system identity -------------------------------------------------
    fs::write(etc.join("hostname"), format!("{}\n", cfg.hostname))
        .map_err(|e| format!("cannot write hostname: {e}"))?;
    log.log_result("etcinit", "hostname", Some(cfg.hostname.clone()))?;

    let tz_path = slot.join(format!("usr/share/zoneinfo/{}", cfg.timezone));
    if !tz_path.is_file() {
        return Err(format!(
            "timezone {0:?} not found in the installed release (expected {1})",
            cfg.timezone,
            tz_path.display()
        ));
    }
    let lt = etc.join("localtime");
    let _ = fs::remove_file(&lt);
    std::os::unix::fs::symlink(tz_path, &lt).map_err(|e| format!("cannot link localtime: {e}"))?;
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
        check_user_name(&u.name)?;
        let shell = u
            .shell
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_SHELL.to_string());
        if !slot
            .join(shell.strip_prefix('/').unwrap_or(&shell))
            .is_file()
        {
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

    // --- services ---------------------------------------------------------
    let wants = etc.join("systemd/system/multi-user.target.wants");
    for unit in &cfg.services {
        check_unit_name(unit)?;
        let shipped = slot.join(format!("usr/lib/systemd/system/{unit}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_names_validated() {
        assert!(check_user_name("tommy").is_ok());
        assert!(check_user_name("a-b_c9").is_ok());
        assert!(check_user_name("").is_err());
        assert!(check_user_name("-root").is_err());
        assert!(check_user_name(".hidden").is_err());
        assert!(check_user_name("bad:name").is_err());
        assert!(check_user_name("root").is_err());
        assert!(check_user_name("nobody").is_err());
    }

    #[test]
    fn unit_names_validated() {
        assert!(check_unit_name("sshd.service").is_ok());
        assert!(check_unit_name("ssh@server.service").is_ok());
        assert!(check_unit_name("no-dot").is_err());
        assert!(check_unit_name("bad;unit.service").is_err());
        assert!(check_unit_name("").is_err());
    }
}
