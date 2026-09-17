//! Ingot declarative installer engine (spec 11.4): parse a TOML
//! install config and drive the install phases - validation, repart
//! partitioning, mkfs, whole-image slot deployment, /var/lib/etc
//! initialization, bootctl install and UKI placement, finalization.
//!
//! Non-interactive: the config is the authorization. `--dry-run`
//! reports the plan without touching the disk.

mod size;
mod version;

fn main() -> std::process::ExitCode {
    // Temporary scaffold entry point; the CLI (T4) lands on top.
    eprintln!("ingot-installer: CLI not yet built");
    std::process::ExitCode::FAILURE
}
