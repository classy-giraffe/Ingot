//! Command-line interface: argument parsing, usage text, defaults.

/// Parsed arguments.
pub struct Args {
    /// Release source: `owner/repo`.
    pub repo: String,
    /// GPG public keyring with the project signing key.
    pub keyring: String,
    /// Releases list URL (the GitHub Releases API, or a `file://`
    /// fixture). Defaults to the API for `--repo`.
    pub api: String,
    /// Per-release asset directory URL, no trailing slash. Defaults
    /// to the GitHub Releases per-release download location for
    /// `--repo`.
    pub asset_base: String,
    /// Write the pin document here instead of stdout.
    pub out: Option<String>,
}

pub const USAGE: &str = "\
ingot-update-helper: discover the latest Ingot release, authenticate it, and emit the pin

Given a release source, selects the latest release (the highest semver
non-prerelease tag from the GitHub Releases API - the date-based
'latest' is not used), authenticates it (the manifest.json and
SHA256SUMS GPG signatures must verify against the project keyring, and
SHA256SUMS must agree with the manifest), and emits a pin document:
the version plus the per-tag, per-asset URLs. The pin is the input the
systemd-sysupdate transfers consume (asset_base = [Source] Path=,
asset names = MatchPattern= expansions, version = 'systemd-sysupdate
update <version>').

Usage: ingot-update-helper [OPTIONS]

Options:
  --repo <OWNER/REPO>     release source (required)
  --keyring <PATH>        GPG public keyring with the project signing key (required)
  --api <URL>             releases list URL
                          (default: https://api.github.com/repos/<repo>/releases;
                          file:// URLs read a local fixture)
  --asset-base <URL>      per-release asset directory, no trailing slash
                          (default: https://github.com/<owner>/<repo>/releases/download)
  --out <PATH>            write the pin document (JSON) to PATH instead of stdout
  -h, --help              print this help
  -V, --version           print the version

Exit codes: 0 ok; 1 discovery, fetch, or authentication failure;
2 usage error.
";

/// Parses argv (without the program name).
pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut repo: Option<String> = None;
    let mut keyring: Option<String> = None;
    let mut api: Option<String> = None;
    let mut asset_base: Option<String> = None;
    let mut out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let mut take_value = |flag: &str| -> Result<String, String> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("ingot-update-helper {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--repo" => repo = Some(take_value(a)?),
            "--keyring" => keyring = Some(take_value(a)?),
            "--api" => api = Some(take_value(a)?),
            "--asset-base" => asset_base = Some(take_value(a)?),
            "--out" => out = Some(take_value(a)?),
            other => {
                return Err(format!("unknown argument '{other}'"));
            }
        }
        i += 1;
    }

    let repo =
        repo.ok_or_else(|| "--repo <OWNER/REPO> is required".to_string())?;
    if !is_repo(&repo) {
        return Err(format!(
            "invalid --repo '{repo}' (expected OWNER/REPO, e.g. classy-giraffe/Ingot)"
        ));
    }
    let keyring =
        keyring.ok_or_else(|| "--keyring <PATH> is required".to_string())?;
    let (owner, name) = repo.split_once('/').unwrap();
    Ok(Args {
        api: api.unwrap_or_else(|| {
            format!("https://api.github.com/repos/{repo}/releases")
        }),
        asset_base: asset_base.unwrap_or_else(|| {
            format!("https://github.com/{owner}/{name}/releases/download")
        }),
        repo,
        keyring,
        out,
    })
}

/// GitHub owner and repo names: alphanumerics and hyphens, no leading
/// or trailing hyphen, at least one character each, exactly one '/'.
fn is_repo(repo: &str) -> bool {
    let (owner, name) = match repo.split_once('/') {
        Some((o, n)) if !n.contains('/') => (o, n),
        _ => return false,
    };
    is_name(owner) && is_name(name)
}

fn is_name(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with('-')
        && !part.ends_with('-')
        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
