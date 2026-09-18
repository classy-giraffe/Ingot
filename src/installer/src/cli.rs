//! Command-line interface: argument parsing, usage text, exit codes.
//! The usage text in `USAGE` is the single source of truth.

use std::path::PathBuf;

pub const USAGE: &str = "Usage: ingot-installer [OPTIONS] <CONFIG>\n\
                         ingot-installer --wizard [OPTIONS]\n\
                         \n\
                         Ingot installer (spec 11.3/11.4): drive the install phases\n\
                         from a declarative TOML config, or collect the config in an\n\
                         interactive terminal wizard and run the same engine. The\n\
                         config is the source of truth in both modes.\n\
                         \n\
                         Engine mode (non-interactive; the config is the authorization):\n\
                         \x20 <CONFIG>            Path to the install config TOML\n\
                         \n\
                         Options:\n\
                         \x20 -h, --help           Print help\n\
                         \x20     --dry-run        Report the plan without touching the disk\n\
                         \x20     --work <DIR>     Working directory [default: /run/ingot-install]\n\
                         \x20 -V, --version        Print version\n\
                         \n\
                         Wizard mode (--wizard; interactive TUI, spec 11.6.2):\n\
                         \x20     --config <PATH>  Write the collected config to PATH\n\
                         \x20                      [default: ingot-install.toml]\n\
                         \x20     --dry-run        Stop after showing the plan (no install)\n\
                         \n\
                         Exit codes: 0 success; 1 execution or verification failure;\n\
                         2 usage or config parse error; 130 aborted by the user.";

pub const DEFAULT_WORK: &str = "/run/ingot-install";
pub const DEFAULT_CONFIG_OUT: &str = "ingot-install.toml";

/// Parsed command line.
#[derive(Debug)]
pub struct Args {
    /// Engine mode: the TOML config that authorizes the install.
    pub config: Option<PathBuf>,
    /// Wizard mode: interactive TUI that writes `config_out`, then
    /// runs the engine on that file.
    pub wizard: bool,
    /// Wizard mode: where the collected config is written.
    pub config_out: PathBuf,
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
    let mut wizard = false;
    let mut config_out = PathBuf::from(DEFAULT_CONFIG_OUT);
    let mut config_out_set = false;
    let mut dry_run = false;
    let mut work = PathBuf::from(DEFAULT_WORK);

    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(CliError::Help),
            "-V" | "--version" => return Err(CliError::Version),
            "--dry-run" => dry_run = true,
            "--wizard" => {
                if wizard {
                    return Err(CliError::Error(
                        "option '--wizard' given more than once".into(),
                    ));
                }
                if config.is_some() {
                    return Err(CliError::Error(wizard_rejects_config()));
                }
                wizard = true;
            }
            "--work" => {
                let dir = it.next().ok_or_else(|| {
                    CliError::Error("--work requires a directory argument".into())
                })?;
                work = PathBuf::from(dir);
            }
            s if s.starts_with("--work=") => {
                work = PathBuf::from(&s["--work=".len()..]);
            }
            "--config" => {
                let path = it.next().ok_or_else(|| {
                    CliError::Error("--config requires a path argument".into())
                })?;
                config_out = PathBuf::from(path);
                config_out_set = true;
            }
            s if s.starts_with("--config=") => {
                config_out = PathBuf::from(&s["--config=".len()..]);
                config_out_set = true;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(CliError::Error(format!(
                    "unknown option {s:?} (see --help)"
                )));
            }
            s => {
                if wizard {
                    return Err(CliError::Error(wizard_rejects_config()));
                }
                if config.is_some() {
                    return Err(CliError::Error(
                        "multiple config paths given (expected exactly one <CONFIG>)".into(),
                    ));
                }
                config = Some(PathBuf::from(s));
            }
        }
    }
    if wizard {
        return Ok(Args {
            config: None,
            wizard,
            config_out,
            dry_run,
            work,
        });
    }
    if config_out_set {
        return Err(CliError::Error(
            "--config is a wizard option (--wizard); engine mode takes <CONFIG>".into(),
        ));
    }
    match config {
        Some(c) => Ok(Args {
            config: Some(c),
            wizard: false,
            config_out,
            dry_run,
            work,
        }),
        None => Err(CliError::Error(
            "missing <CONFIG> argument (or use --wizard; see --help)".into(),
        )),
    }
}

fn wizard_rejects_config() -> String {
    "wizard mode takes no <CONFIG> argument (the wizard writes the config itself)".to_string()
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
        assert_eq!(a.config.as_ref().unwrap().to_str(), Some("conf.toml"));
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
        assert_eq!(a.config.as_ref().unwrap().to_str(), Some("c.toml"));
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

    #[test]
    fn wizard_mode_parsing() {
        // wizard mode: no <CONFIG>, output path and work default
        let a = parse(v("--wizard")).unwrap();
        assert!(a.wizard);
        assert_eq!(a.config, None);
        assert_eq!(a.config_out.to_str(), Some(DEFAULT_CONFIG_OUT));
        assert_eq!(a.work.to_str(), Some(DEFAULT_WORK));
        assert!(!a.dry_run);

        // --config path and --work passthrough; equals form too
        let a = parse(v("--wizard --config /x/install.toml --work /w")).unwrap();
        assert_eq!(a.config_out.to_str(), Some("/x/install.toml"));
        assert_eq!(a.work.to_str(), Some("/w"));
        let a = parse(v("--config=/x.toml --wizard")).unwrap();
        assert_eq!(a.config_out.to_str(), Some("/x.toml"));

        // --dry-run is allowed with the wizard (stop after the plan)
        let a = parse(v("--dry-run --wizard")).unwrap();
        assert!(a.dry_run);
        assert!(a.wizard);
    }

    #[test]
    fn wizard_mode_rejects_a_config_argument() {
        for bad in ["--wizard conf.toml", "conf.toml --wizard", "conf.toml --config /x.toml"] {
            match parse(v(bad)) {
                Err(CliError::Error(e)) => {
                    assert!(!e.is_empty(), "{bad:?}");
                }
                other => panic!("{bad:?} -> {other:?}"),
            }
        }
    }

    #[test]
    fn engine_mode_unchanged() {
        let a = parse(v("conf.toml")).unwrap();
        assert!(!a.wizard);
        assert_eq!(a.config.as_ref().unwrap().to_str(), Some("conf.toml"));
        assert_eq!(a.config_out.to_str(), Some(DEFAULT_CONFIG_OUT));
    }
}
