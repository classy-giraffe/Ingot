//! Command-line interface: argument parsing, usage text, exit codes.
//!
//! ```text
//! ingot-installer [OPTIONS] <CONFIG>
//!
//! OPTIONS:
//!   -h, --help       Print help
//!       --dry-run    Report the plan without touching the disk
//!       --work <DIR> Working directory (default /run/ingot-install)
//!   -V, --version    Print version
//!
//! Exit codes: 0 success (install done, or plan reported with
//! --dry-run); 1 execution or verification failure; 2 usage or
//! config parse error.
//! ```

use std::path::PathBuf;

pub const USAGE: &str = "Usage: ingot-installer [OPTIONS] <CONFIG>\n\
                         \n\
                         Declarative Ingot installer (spec 11.4): parse a TOML install\n\
                         config and drive the install phases. Non-interactive: the\n\
                         config is the authorization.\n\
                         \n\
                         Arguments:\n\
                         \x20 <CONFIG>            Path to the install config TOML\n\
                         \n\
                         Options:\n\
                         \x20 -h, --help           Print help\n\
                         \x20     --dry-run        Report the plan without touching the disk\n\
                         \x20     --work <DIR>     Working directory [default: /run/ingot-install]\n\
                         \x20 -V, --version        Print version\n\
                         \n\
                         Exit codes: 0 success; 1 execution or verification failure;\n\
                         2 usage or config parse error.";

pub const DEFAULT_WORK: &str = "/run/ingot-install";

/// Parsed command line.
#[derive(Debug)]
pub struct Args {
    pub config: PathBuf,
    pub dry_run: bool,
    pub work: PathBuf,
}

/// What went wrong with the command line.
#[derive(Debug)]
pub enum CliError {
    /// A real usage error (exit code 2, usage printed on demand).
    Error(String),
    /// --help: print the usage and exit 0.
    Help,
    /// --version: print the version and exit 0.
    Version,
}

/// Parses argv (without the program name).
pub fn parse<I>(args: I) -> Result<Args, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut config: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut work = PathBuf::from(DEFAULT_WORK);

    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(CliError::Help),
            "-V" | "--version" => return Err(CliError::Version),
            "--dry-run" => dry_run = true,
            "--work" => {
                let dir = it.next().ok_or_else(|| {
                    CliError::Error("--work requires a directory argument".into())
                })?;
                work = PathBuf::from(dir);
            }
            s if s.starts_with("--work=") => {
                work = PathBuf::from(&s["--work=".len()..]);
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(CliError::Error(format!(
                    "unknown option {s:?} (see --help)"
                )));
            }
            s => {
                if config.is_some() {
                    return Err(CliError::Error(
                        "multiple config paths given (expected exactly one <CONFIG>)".into(),
                    ));
                }
                config = Some(PathBuf::from(s));
            }
        }
    }
    match config {
        Some(c) => Ok(Args {
            config: c,
            dry_run,
            work,
        }),
        None => Err(CliError::Error(
            "missing <CONFIG> argument (see --help)".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_full_args() {
        let a = parse(v("conf.toml --dry-run --work /tmp/w")).unwrap();
        assert_eq!(a.config.to_str(), Some("conf.toml"));
        assert!(a.dry_run);
        assert_eq!(a.work.to_str(), Some("/tmp/w"));
    }

    #[test]
    fn parses_work_equals_form() {
        let a = parse(v("--work=/tmp/w conf.toml")).unwrap();
        assert_eq!(a.work.to_str(), Some("/tmp/w"));
        assert!(!a.dry_run);
    }

    #[test]
    fn defaults() {
        let a = parse(v("c.toml")).unwrap();
        assert_eq!(a.work.to_str(), Some(DEFAULT_WORK));
        assert!(!a.dry_run);
    }

    #[test]
    fn help_and_version() {
        assert!(matches!(parse(v("-h")), Err(CliError::Help)));
        assert!(matches!(parse(v("--help")), Err(CliError::Help)));
        assert!(matches!(parse(v("-V")), Err(CliError::Version)));
        assert!(matches!(parse(v("--version")), Err(CliError::Version)));
    }

    #[test]
    fn usage_errors() {
        for bad in ["", "a b", "--nope c.toml", "--work", "c.toml --work"] {
            match parse(v(bad)) {
                Err(CliError::Error(e)) => assert!(!e.is_empty()),
                other => panic!("{bad:?} -> {other:?}"),
            }
        }
    }
}
