//! CLI contract tests for the installer binary: arguments, exit
//! codes, and stderr messages (the wizard's interactive behavior is
//! covered by the PTY-driven harness scenario).

use std::process::Command;

fn installer() -> &'static str {
    env!("CARGO_BIN_EXE_ingot-installer")
}

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Outcome {
    let out = Command::new(installer())
        .args(args)
        .output()
        .expect("running ingot-installer");
    Outcome {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

#[test]
fn wizard_requires_an_interactive_terminal() {
    // piped stdio: not a tty
    let o = run(&["--wizard"]);
    assert_eq!(o.code, Some(2));
    assert!(o.stderr.contains("interactive terminal"), "{}", o.stderr);
}

#[test]
fn wizard_rejects_a_config_argument() {
    let o = run(&["--wizard", "config.toml"]);
    assert_eq!(o.code, Some(2));
    assert!(
        o.stderr.contains("wizard mode takes no <CONFIG>"),
        "{}",
        o.stderr
    );
}

#[test]
fn engine_mode_missing_config_is_a_usage_error() {
    let o = run(&[]);
    assert_eq!(o.code, Some(2));
    assert!(o.stderr.contains("missing <CONFIG>"), "{}", o.stderr);
    assert!(o.stderr.contains("--wizard"), "{}", o.stderr);
}

#[test]
fn config_option_is_wizard_only() {
    let o = run(&["config.toml", "--config", "other.toml"]);
    assert_eq!(o.code, Some(2));
    assert!(o.stderr.contains("--config"), "{}", o.stderr);
}

#[test]
fn help_and_version_exit_zero() {
    let o = run(&["--help"]);
    assert_eq!(o.code, Some(0));
    assert!(o.stdout.contains("--wizard"), "{}", o.stdout);
    assert!(o.stdout.contains("130"), "{}", o.stdout);
    let o = run(&["-V"]);
    assert_eq!(o.code, Some(0));
    assert!(o.stdout.contains("ingot-installer"), "{}", o.stdout);
}

#[test]
fn a_missing_config_file_is_a_parse_error() {
    let o = run(&["/no/such/config.toml"]);
    assert_eq!(o.code, Some(2));
    assert!(
        o.stderr.contains("cannot read config /no/such/config.toml"),
        "{}",
        o.stderr
    );
}
