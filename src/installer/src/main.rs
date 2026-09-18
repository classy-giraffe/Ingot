//! Ingot installer (spec 11): one execution engine, two front-ends
//! (spec 11.3). Engine mode parses a TOML install config (the
//! authorization) and drives the install phases - validation, repart
//! partitioning, mkfs, whole-image slot deployment, /var/lib/etc
//! initialization, bootctl install and UKI placement, finalization.
//! Wizard mode collects the same categories in a terminal TUI,
//! writes the TOML config, shows the plan, requires explicit
//! confirmation, and runs the same engine on that file.
//!
//! Exit codes: 0 success (install complete, or dry-run plan
//! reported); 1 execution or verification failure; 2 usage or
//! config parse error; 130 aborted by the user (wizard mode).

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

    if args.wizard {
        use crossterm::tty::IsTty;
        if !std::io::stdin().is_tty() || !std::io::stdout().is_tty() {
            eprintln!("ingot-installer: --wizard requires an interactive terminal (use the declarative config mode for scripted installs)");
            return std::process::ExitCode::from(2);
        }
        return match wizard::ui::run(&args.config_out, &args.work, args.dry_run) {
            wizard::ui::Outcome::Done => std::process::ExitCode::SUCCESS,
            wizard::ui::Outcome::Aborted => std::process::ExitCode::from(130),
            wizard::ui::Outcome::Failed(e) => {
                eprintln!("ingot-installer: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    let config_path = args
        .config
        .expect("engine mode always carries a config path");
    let text = match std::fs::read_to_string(&config_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "ingot-installer: cannot read config {}: {e}",
                config_path.display()
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
