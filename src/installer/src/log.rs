//! Install event log (spec 11.6.4).
//!
//! The engine appends one JSON line per action to `install.log`. The
//! working copy lives in the working directory for the whole run; on
//! success (and on failure, if the var mount is up) it is copied to
//! the target's `/var/lib/ingot/install.log` so the log is retrievable
//! from the installed system.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(serde::Serialize)]
struct Event {
    t: u64,
    phase: &'static str,
    event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// A JSONL install log.
#[derive(Debug)]
pub struct InstallLog {
    path: PathBuf,
}

impl InstallLog {
    /// Opens (creating the file and parent dirs) for appending.
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }

    /// `log` with io errors mapped to `String`, for engine callers.
    pub fn log_result(
        &self,
        phase: &'static str,
        event: &str,
        detail: Option<String>,
    ) -> Result<(), String> {
        self.log(phase, event, detail).map_err(|e| e.to_string())
    }
    /// Appends one event as a JSON line.
    pub fn log(
        &self,
        phase: &'static str,
        event: &str,
        detail: Option<String>,
    ) -> std::io::Result<()> {
        let ev = Event {
            t: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            phase,
            event: event.to_string(),
            detail,
        };
        let mut line = serde_json::to_string(&ev)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        line.push('\n');
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(line.as_bytes())
    }

    /// The log file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_jsonl_events() {
        let dir = std::env::temp_dir().join(format!("ingot-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log = InstallLog::open(dir.join("nested/install.log")).unwrap();
        log.log("validate", "config-ok", None).unwrap();
        log.log("deploy", "slot-a", Some("308M".into())).unwrap();

        let text = fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let e1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(e1["phase"], "validate");
        assert_eq!(e1["event"], "config-ok");
        assert!(e1.get("detail").is_none());
        let e2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(e2["detail"], "308M");
        assert!(e2["t"].as_u64().unwrap() > 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
