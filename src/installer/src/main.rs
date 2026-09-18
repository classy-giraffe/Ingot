//! Ingot declarative installer engine (spec 11.4): parse a TOML
//! install config and drive the install phases - validation, repart
//! partitioning, mkfs, whole-image slot deployment, /var/lib/etc
//! initialization, bootctl install and UKI placement, finalization.
//!
//! Non-interactive: the config is the authorization. `--dry-run`
//! reports the plan without touching the disk.
//!
//! Exit codes: 0 success (install complete, or dry-run plan
//! reported); 1 execution or verification failure; 2 usage or
//! config parse error.

mod boot;
mod cli;
mod config;
mod deploy;
mod engine;
mod etcinit;
mod finalize;
mod layout;
mod log;
mod repart;
mod size;
mod source;
mod sshkey;
mod target;
mod ukify;
mod version;
mod wizard;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse(argv) {
        Ok(a) => a,
        Err(cli::CliError::Help) => {
            println!("{}", cli::USAGE);
            return std::process::ExitCode::SUCCESS;
        }
        Err(cli::CliError::Version) => {
            println!("ingot-installer {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        Err(cli::CliError::Error(e)) => {
            eprintln!("ingot-installer: {e}");
            eprintln!("{}", cli::USAGE);
            return std::process::ExitCode::from(2);
        }
    };

    let text = match std::fs::read_to_string(&args.config) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "ingot-installer: cannot read config {}: {e}",
                args.config.display()
            );
            return std::process::ExitCode::from(2);
        }
    };

    let cfg = match config::parse(&text) {
        Ok(c) => c,
        Err(errs) => {
            for e in errs {
                eprintln!("config error: {e}");
            }
            return std::process::ExitCode::from(2);
        }
    };

    let result = if args.dry_run {
        engine::dry_run(cfg)
    } else {
        engine::run(cfg, &args.work).map(|_| String::new())
    };

    match result {
        Ok(report) => {
            if !report.is_empty() {
                println!("{report}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ingot-installer: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
