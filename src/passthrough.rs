use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// Resolve a binary by name using env override, then PATH.
pub fn resolve_binary(name: &str) -> Result<PathBuf> {
    let env_key = format!("GREENTIC_DEV_BIN_{}", name.replace('-', "_").to_uppercase());
    if let Ok(path) = env::var(&env_key) {
        let pb = PathBuf::from(path);
        if pb.exists() {
            return Ok(pb);
        }
        bail!("{env_key} points to non-existent binary: {}", pb.display());
    }

    if let Ok(path) = which::which(name) {
        return Ok(path);
    }

    bail!(
        "failed to find `{name}` in PATH; set {env_key}, install `{name}` with cargo binstall, or run `greentic-dev install tools` (`--latest` to force-refresh)"
    )
}

pub fn run_passthrough(bin: &Path, args: &[OsString], verbose: bool) -> Result<ExitStatus> {
    if verbose {
        eprintln!("greentic-dev passthrough -> {} {:?}", bin.display(), args);
        let _ = Command::new(bin)
            .arg("--version")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();
    }

    Command::new(bin)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow!("failed to execute {}: {e}", bin.display()))
}

#[derive(Clone, Copy)]
struct InstallSpec {
    crate_name: &'static str,
    bin_name: &'static str,
}

const DELEGATED_INSTALL_SPECS: [InstallSpec; 7] = [
    InstallSpec {
        crate_name: "greentic-component",
        bin_name: "greentic-component",
    },
    InstallSpec {
        crate_name: "greentic-flow",
        bin_name: "greentic-flow",
    },
    InstallSpec {
        crate_name: "greentic-pack",
        bin_name: "greentic-pack",
    },
    InstallSpec {
        crate_name: "greentic-runner",
        bin_name: "greentic-runner",
    },
    InstallSpec {
        crate_name: "greentic-runner",
        bin_name: "greentic-runner-cli",
    },
    InstallSpec {
        crate_name: "greentic-gui",
        bin_name: "greentic-gui",
    },
    InstallSpec {
        crate_name: "greentic-secrets",
        bin_name: "greentic-secrets",
    },
];

pub fn install_all_delegated_tools(latest: bool) -> Result<()> {
    ensure_cargo_binstall()?;
    for spec in DELEGATED_INSTALL_SPECS {
        install_with_binstall(spec, latest)?;
    }
    Ok(())
}

fn install_with_binstall(spec: InstallSpec, force_latest: bool) -> Result<()> {
    eprintln!(
        "greentic-dev: installing `{}` from crate `{}` via cargo binstall...",
        spec.bin_name, spec.crate_name
    );

    let mut cmd = Command::new("cargo");
    cmd.arg("binstall")
        .arg("-y")
        .arg("--locked")
        .arg(spec.crate_name)
        .arg("--bin")
        .arg(spec.bin_name);
    if force_latest {
        cmd.arg("--force");
    }

    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| "failed to execute `cargo binstall`")?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "`cargo binstall` failed while installing `{}` (crate `{}`), exit code {:?}",
            spec.bin_name,
            spec.crate_name,
            status.code()
        );
    }
}

fn ensure_cargo_binstall() -> Result<()> {
    let installed_version = installed_cargo_binstall_version()?;
    if installed_version.is_none() {
        eprintln!("greentic-dev: installing `cargo-binstall` via cargo...");
        return install_cargo_binstall();
    }

    let installed_version = installed_version.expect("checked is_some above");
    match latest_cargo_binstall_version() {
        Ok(latest_version) => {
            if installed_version >= latest_version {
                return Ok(());
            }

            eprintln!(
                "greentic-dev: updating `cargo-binstall` from {} to {} via cargo...",
                installed_version, latest_version
            );
            install_cargo_binstall()
        }
        Err(err) => {
            eprintln!(
                "greentic-dev: failed to check latest `cargo-binstall` version ({err}); continuing with installed version {installed_version}."
            );
            Ok(())
        }
    }
}

fn install_cargo_binstall() -> Result<()> {
    let status = Command::new("cargo")
        .arg("install")
        .arg("cargo-binstall")
        .arg("--locked")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| "failed to execute `cargo install cargo-binstall --locked`")?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "failed to install cargo-binstall; `cargo install cargo-binstall --locked` exit code {:?}",
            status.code()
        );
    }
}

fn installed_cargo_binstall_version() -> Result<Option<Version>> {
    let output = Command::new("cargo")
        .arg("binstall")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8(output.stdout)
        .context("`cargo binstall --version` returned non-UTF8 output")?;
    parse_installed_cargo_binstall_version(&stdout)
}

fn latest_cargo_binstall_version() -> Result<Version> {
    let output = Command::new("cargo")
        .arg("search")
        .arg("cargo-binstall")
        .arg("--limit")
        .arg("1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .with_context(|| "failed to execute `cargo search cargo-binstall --limit 1`")?;
    if !output.status.success() {
        bail!(
            "`cargo search cargo-binstall --limit 1` failed with exit code {:?}",
            output.status.code()
        );
    }

    let stdout = String::from_utf8(output.stdout)
        .context("`cargo search cargo-binstall --limit 1` returned non-UTF8 output")?;
    parse_latest_cargo_binstall_version(&stdout)
}

fn parse_installed_cargo_binstall_version(stdout: &str) -> Result<Option<Version>> {
    let line = stdout.lines().next().unwrap_or_default();
    let maybe_version = line
        .split_whitespace()
        .find_map(|token| Version::parse(token.trim_start_matches('v')).ok());
    Ok(maybe_version)
}

fn parse_latest_cargo_binstall_version(stdout: &str) -> Result<Version> {
    let first_line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| anyhow!("`cargo search cargo-binstall --limit 1` returned no results"))?;
    let (_, rhs) = first_line
        .split_once('=')
        .ok_or_else(|| anyhow!("unexpected cargo search output: {first_line}"))?;
    let quoted = rhs
        .split('#')
        .next()
        .map(str::trim)
        .ok_or_else(|| anyhow!("unexpected cargo search output: {first_line}"))?;
    let version_text = quoted.trim_matches('"');
    Version::parse(version_text)
        .with_context(|| format!("failed to parse cargo-binstall version from `{first_line}`"))
}

#[cfg(test)]
mod tests {
    use super::{
        DELEGATED_INSTALL_SPECS, parse_installed_cargo_binstall_version,
        parse_latest_cargo_binstall_version,
    };

    #[test]
    fn delegated_install_specs_include_runner_cli() {
        let found = DELEGATED_INSTALL_SPECS.iter().any(|spec| {
            spec.bin_name == "greentic-runner-cli" && spec.crate_name == "greentic-runner"
        });
        assert!(found);
    }

    #[test]
    fn parse_installed_binstall_version_line() {
        let parsed = parse_installed_cargo_binstall_version("cargo-binstall 1.15.7\n")
            .expect("parse should succeed")
            .expect("version should exist");
        assert_eq!(parsed.to_string(), "1.15.7");
    }

    #[test]
    fn parse_latest_binstall_version_line() {
        let parsed = parse_latest_cargo_binstall_version(
            "cargo-binstall = \"1.15.7\"    # Binary installation for rust projects\n",
        )
        .expect("parse should succeed");
        assert_eq!(parsed.to_string(), "1.15.7");
    }
}
