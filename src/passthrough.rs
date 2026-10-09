use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::toolchain_catalogue::{GREENTIC_EXTERNAL_TOOL_PACKAGES, GREENTIC_TOOLCHAIN_PACKAGES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolchainChannel {
    Stable,
    Development,
    Rnd,
}

impl ToolchainChannel {
    pub fn from_executable_name(name: &str) -> Self {
        let stem = name.strip_suffix(".exe").unwrap_or(name);
        if stem == "greentic-dev-dev" {
            Self::Development
        } else if stem == "greentic-dev-rnd" {
            Self::Rnd
        } else {
            Self::Stable
        }
    }
}

pub fn current_toolchain_channel() -> ToolchainChannel {
    let executable_name = env::args_os()
        .next()
        .and_then(|arg| PathBuf::from(arg).file_name().map(|name| name.to_owned()))
        .or_else(|| {
            env::current_exe()
                .ok()
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
        });
    executable_name
        .as_deref()
        .and_then(|name| name.to_str())
        .map(ToolchainChannel::from_executable_name)
        .unwrap_or(ToolchainChannel::Stable)
}

pub fn delegated_binary_name(name: &str) -> String {
    delegated_binary_name_for_channel(name, current_toolchain_channel())
}

pub fn delegated_binary_name_for_channel(name: &str, channel: ToolchainChannel) -> String {
    match channel {
        ToolchainChannel::Stable => name.to_string(),
        ToolchainChannel::Development => suffixed_binary_name(name, "dev"),
        ToolchainChannel::Rnd => suffixed_binary_name(name, "rnd"),
    }
}

fn suffixed_binary_name(name: &str, suffix: &str) -> String {
    if name == "greentic-dev" {
        return format!("greentic-dev-{suffix}");
    }
    let suffix = format!("-{suffix}");
    if name.ends_with(&suffix) {
        name.to_string()
    } else {
        format!("{name}{suffix}")
    }
}

/// Resolve a binary by name using env override, then PATH.
pub fn resolve_binary(name: &str) -> Result<PathBuf> {
    resolve_binary_for_channel(name, current_toolchain_channel())
}

pub fn resolve_binary_for_channel(name: &str, channel: ToolchainChannel) -> Result<PathBuf> {
    let locale = crate::i18n::select_locale(None);
    let resolved_name = delegated_binary_name_for_channel(name, channel);
    let env_key = format!(
        "GREENTIC_DEV_BIN_{}",
        resolved_name.replace('-', "_").to_uppercase()
    );
    if let Ok(path) = env::var(&env_key) {
        let pb = PathBuf::from(path);
        if pb.exists() {
            return Ok(pb);
        }
        bail!(
            "{}",
            crate::i18n::tf(
                &locale,
                "runtime.passthrough.error.env_binary_missing",
                &[
                    ("env_key", env_key.clone()),
                    ("path", pb.display().to_string()),
                ],
            )
        );
    }

    if let Ok(path) = which::which(&resolved_name) {
        return Ok(path);
    }

    bail!(
        "{}",
        crate::i18n::tf(
            &locale,
            "runtime.passthrough.error.binary_not_found",
            &[("name", resolved_name), ("env_key", env_key)],
        )
    )
}

/// Environment-override key for an external tool, e.g. `greentic-mcp-gen`
/// → `GREENTIC_DEV_BIN_GREENTIC_MCP_GEN`.
pub(crate) fn external_tool_env_key(name: &str) -> String {
    format!("GREENTIC_DEV_BIN_{}", name.replace('-', "_").to_uppercase())
}

/// Resolve an external (non-Greentic-channel) tool binary by its plain name.
///
/// Unlike [`resolve_binary`], this never appends the toolchain channel suffix
/// (`-dev`/`-rnd`): external tools such as `greentic-mcp-gen` ship a single,
/// unsuffixed binary. Resolution order: `GREENTIC_DEV_BIN_<NAME>` env override,
/// then `PATH`.
pub fn resolve_external_tool(name: &str) -> Result<PathBuf> {
    let locale = crate::i18n::select_locale(None);
    let env_key = external_tool_env_key(name);
    if let Ok(path) = env::var(&env_key) {
        let pb = PathBuf::from(path);
        if pb.exists() {
            return Ok(pb);
        }
        bail!(
            "{}",
            crate::i18n::tf(
                &locale,
                "runtime.passthrough.error.env_binary_missing",
                &[
                    ("env_key", env_key.clone()),
                    ("path", pb.display().to_string()),
                ],
            )
        );
    }

    if let Ok(path) = which::which(name) {
        return Ok(path);
    }

    bail!(
        "{}",
        crate::i18n::tf(
            &locale,
            "runtime.passthrough.error.binary_not_found",
            &[("name", name.to_string()), ("env_key", env_key)],
        )
    )
}

pub fn run_passthrough(bin: &Path, args: &[OsString], verbose: bool) -> Result<ExitStatus> {
    let locale = crate::i18n::select_locale(None);
    if verbose {
        eprintln!(
            "{}",
            crate::i18n::tf(
                &locale,
                "runtime.passthrough.debug.exec",
                &[
                    ("bin", bin.display().to_string()),
                    ("args", format!("{args:?}")),
                ],
            )
        );
        // Accepted risk: delegated Greentic tool path is resolved from fixed tool names or explicit local override; no shell is invoked.
        // foxguard: ignore[rs/no-command-injection]
        let _ = Command::new(bin)
            .arg("--version")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();
    }

    // Accepted risk: passthrough intentionally executes a resolved Greentic tool binary with argv, never through a shell.
    // foxguard: ignore[rs/no-command-injection]
    Command::new(bin)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            anyhow!(crate::i18n::tf(
                &locale,
                "runtime.passthrough.error.execute",
                &[("bin", bin.display().to_string()), ("error", e.to_string())],
            ))
        })
}

pub fn install_all_delegated_tools(latest: bool, locale: &str) -> Result<()> {
    ensure_cargo_binstall()?;
    let mut failed: Vec<String> = Vec::new();
    let channel = current_toolchain_channel();
    // The research (`rnd`) lane publishes `<crate>-rnd` at `X.Y.Z-research`
    // PRERELEASE versions. `cargo binstall`/cargo will not select a pre-release
    // without an explicit `--version`, so for the Rnd channel we resolve the
    // latest research version per crate and pin it. Stable/Development publish
    // regular releases that binstall picks up without a version.
    let resolver = (channel == ToolchainChannel::Rnd)
        .then(crate::release_cmd::CratesIoApiVersionResolver::default);
    for package in GREENTIC_TOOLCHAIN_PACKAGES {
        let crate_name = delegated_binary_name_for_channel(package.crate_name, channel);
        let version: Option<String> = match resolver.as_ref() {
            // Research channel: resolve the `-rnd` prerelease to pin. Tools with
            // no research build (404) come back `Absent` — skip them with a note
            // instead of aborting the whole install, since only start/runner/
            // setup ship `-research` builds. Use the stable channel for the rest.
            Some(resolver) => {
                match crate::release_cmd::CrateVersionResolver::resolve_research_version(
                    resolver,
                    &crate_name,
                )
                .with_context(|| {
                    format!(
                        "failed to resolve research version for `{crate_name}` \
                         (gtc-research install needs an explicit pre-release version)"
                    )
                })? {
                    crate::release_cmd::ResearchVersion::Pinned(version) => Some(version),
                    crate::release_cmd::ResearchVersion::Absent => {
                        eprintln!(
                            "note: `{crate_name}` has no research build on crates.io; \
                             skipping it on the research toolchain (use the stable \
                             channel for this tool)"
                        );
                        continue;
                    }
                }
            }
            None => None,
        };
        for bin_name in package.bins {
            let bin = delegated_binary_name_for_channel(bin_name, channel);
            // Collected, not propagated. With `?` the loop stopped at the
            // first crate binstall could not resolve, having ALREADY replaced
            // every binary before it in the list — so a single unpublishable
            // crate left the toolchain half-moved and the command reporting
            // only the crate that failed. That is the same failure the
            // external-tools loop below was fixed for; the difference is only
            // that these are required, so the run still ends in an error.
            //
            // It matters most on the development channel: crates.io has
            // carried no Greentic dev build since the publishing account was
            // locked on 2026-09-08, so binstall there resolves whatever older
            // version the index still knows. Finishing the list is what keeps
            // one frozen crate from deciding how far the rest got.
            if let Err(err) =
                install_with_binstall(&crate_name, &bin, latest, version.as_deref(), locale)
            {
                eprintln!("error: `{bin}` (crate `{crate_name}`) could not be installed: {err}");
                failed.push(bin);
            }
        }
    }
    // External tools ship a single unsuffixed binary — install by plain name.
    //
    // A failure here MUST NOT abort the run, and must not even be counted.
    // `greentic-mcp-generator` is not published to crates.io at all — it ships
    // as a private GitHub release and reaches a customer through
    // `install --tenant` — so `cargo binstall` can never resolve it and exits
    // 76 every time. With `?`, that took the whole command down, and at the
    // time `install --tenant` called this before fetching a single tenant
    // artifact, so every tenant install failed having installed nothing, with
    // an error naming a crate the operator could do nothing about. (That
    // second half no longer applies: `install --tenant` installs tenant
    // artifacts only — see the comment at its call site in `install.rs`.)
    //
    // The core toolchain above is still REQUIRED and still ends the run in an
    // error; the difference is that it now finishes the list first, so one
    // unresolvable crate does not decide how far the rest got. These are
    // optional, so they are skipped with a note and never reach `failed`.
    for package in GREENTIC_EXTERNAL_TOOL_PACKAGES {
        for bin_name in package.bins {
            if let Err(err) =
                install_with_binstall(package.crate_name, bin_name, latest, None, locale)
            {
                eprintln!(
                    "note: optional external tool `{bin_name}` (crate `{}`) is unavailable; \
                     skipping it: {err}",
                    package.crate_name
                );
            }
        }
    }

    if !failed.is_empty() {
        // Named individually rather than as a count: on the development
        // channel the usual cause is that crates.io has carried no Greentic
        // dev build since the publishing account was locked on 2026-09-08, and
        // which binaries that leaves behind is the whole of what an operator
        // needs to know to side-load them from the GitHub release archives.
        anyhow::bail!(
            "could not install {} required toolchain binaries: {}",
            failed.len(),
            failed.join(", ")
        );
    }
    Ok(())
}

fn install_with_binstall(
    crate_name: &str,
    bin_name: &str,
    force_latest: bool,
    version: Option<&str>,
    locale: &str,
) -> Result<()> {
    eprintln!(
        "{}",
        crate::i18n::tf(
            locale,
            "runtime.tools.install.installing",
            &[
                ("bin_name", bin_name.to_string()),
                ("crate_name", crate_name.to_string()),
            ],
        )
    );

    let mut cmd = Command::new("cargo");
    cmd.args(binstall_args(crate_name, bin_name, force_latest, version));

    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| crate::i18n::t(locale, "runtime.tools.install.error.execute_binstall"))?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "{}",
            crate::i18n::tf(
                locale,
                "runtime.tools.install.error.binstall_failed",
                &[
                    ("bin_name", bin_name.to_string()),
                    ("crate_name", crate_name.to_string()),
                    ("exit_code", format!("{:?}", status.code())),
                ],
            )
        );
    }
}

fn binstall_args(
    crate_name: &str,
    bin_name: &str,
    force_latest: bool,
    version: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "binstall".to_string(),
        "-y".to_string(),
        "--locked".to_string(),
        "--maximum-resolution-timeout".to_string(),
        "60".to_string(),
        crate_name.to_string(),
        "--bin".to_string(),
        bin_name.to_string(),
    ];
    // Pre-release (`X.Y.Z-research`) `-rnd` crates need an explicit pinned
    // version; binstall will not select a pre-release otherwise.
    if let Some(version) = version {
        args.push("--version".to_string());
        args.push(version.to_string());
    }
    if force_latest {
        args.push("--force".to_string());
    }
    args
}

fn ensure_cargo_binstall() -> Result<()> {
    let locale = crate::i18n::select_locale(None);
    let installed_version = installed_cargo_binstall_version()?;
    if installed_version.is_none() {
        eprintln!(
            "{}",
            crate::i18n::t(&locale, "runtime.tools.install.installing_binstall")
        );
        return install_cargo_binstall();
    }

    let installed_version = installed_version.expect("checked is_some above");
    match latest_cargo_binstall_version() {
        Ok(latest_version) => {
            if installed_version >= latest_version {
                return Ok(());
            }

            eprintln!(
                "{}",
                crate::i18n::tf(
                    &locale,
                    "runtime.tools.install.updating_binstall",
                    &[
                        ("installed_version", installed_version.to_string()),
                        ("latest_version", latest_version.to_string()),
                    ],
                )
            );
            install_cargo_binstall()
        }
        Err(err) => {
            eprintln!(
                "{}",
                crate::i18n::tf(
                    &locale,
                    "runtime.tools.install.warn.latest_check_failed",
                    &[
                        ("error", err.to_string()),
                        ("installed_version", installed_version.to_string()),
                    ],
                )
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
        .with_context(|| {
            crate::i18n::t(
                &crate::i18n::select_locale(None),
                "runtime.tools.install.error.execute_install_binstall",
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        let locale = crate::i18n::select_locale(None);
        bail!(
            "{}",
            crate::i18n::tf(
                &locale,
                "runtime.tools.install.error.install_binstall_failed",
                &[("exit_code", format!("{:?}", status.code()))],
            )
        );
    }
}

fn installed_cargo_binstall_version() -> Result<Option<Version>> {
    let output = Command::new("cargo")
        .arg("binstall")
        .arg("-V")
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

    let stdout =
        String::from_utf8(output.stdout).context("`cargo binstall -V` returned non-UTF8 output")?;
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
        ToolchainChannel, binstall_args, delegated_binary_name_for_channel, external_tool_env_key,
        parse_installed_cargo_binstall_version, parse_latest_cargo_binstall_version,
        resolve_external_tool,
    };
    use crate::toolchain_catalogue::GREENTIC_TOOLCHAIN_PACKAGES;

    #[test]
    fn delegated_install_catalogue_includes_runner() {
        let found = GREENTIC_TOOLCHAIN_PACKAGES.iter().any(|package| {
            package.crate_name == "greentic-runner" && package.bins.contains(&"greentic-runner")
        });
        assert!(found);
    }

    #[test]
    fn binstall_args_include_force_only_when_latest_requested() {
        assert_eq!(
            binstall_args("greentic-runner", "greentic-runner", false, None),
            vec![
                "binstall",
                "-y",
                "--locked",
                "--maximum-resolution-timeout",
                "60",
                "greentic-runner",
                "--bin",
                "greentic-runner"
            ]
        );
        assert_eq!(
            binstall_args("greentic-runner", "greentic-runner", true, None),
            vec![
                "binstall",
                "-y",
                "--locked",
                "--maximum-resolution-timeout",
                "60",
                "greentic-runner",
                "--bin",
                "greentic-runner",
                "--force"
            ]
        );
    }

    #[test]
    fn binstall_args_pin_version_for_rnd_prerelease() {
        // The `-rnd` lane publishes `X.Y.Z-research` PRERELEASES; binstall needs
        // an explicit `--version` to select them.
        assert_eq!(
            binstall_args(
                "greentic-start-rnd",
                "greentic-start-rnd",
                false,
                Some("1.2.0-research.1"),
            ),
            vec![
                "binstall",
                "-y",
                "--locked",
                "--maximum-resolution-timeout",
                "60",
                "greentic-start-rnd",
                "--bin",
                "greentic-start-rnd",
                "--version",
                "1.2.0-research.1",
            ]
        );
    }

    #[test]
    fn executable_name_selects_toolchain_channel() {
        assert_eq!(
            ToolchainChannel::from_executable_name("greentic-dev"),
            ToolchainChannel::Stable
        );
        assert_eq!(
            ToolchainChannel::from_executable_name("greentic-dev-dev"),
            ToolchainChannel::Development
        );
        assert_eq!(
            ToolchainChannel::from_executable_name("greentic-dev-dev.exe"),
            ToolchainChannel::Development
        );
        assert_eq!(
            ToolchainChannel::from_executable_name("greentic-dev-rnd"),
            ToolchainChannel::Rnd
        );
        assert_eq!(
            ToolchainChannel::from_executable_name("greentic-dev-rnd.exe"),
            ToolchainChannel::Rnd
        );
    }

    #[test]
    fn development_channel_uses_dev_binary_names() {
        assert_eq!(
            delegated_binary_name_for_channel("greentic-pack", ToolchainChannel::Development),
            "greentic-pack-dev"
        );
        assert_eq!(
            delegated_binary_name_for_channel("greentic-runner-cli", ToolchainChannel::Development),
            "greentic-runner-cli-dev"
        );
        assert_eq!(
            delegated_binary_name_for_channel("greentic-pack-dev", ToolchainChannel::Development),
            "greentic-pack-dev"
        );
    }

    #[test]
    fn rnd_channel_uses_rnd_binary_names() {
        assert_eq!(
            delegated_binary_name_for_channel("greentic-pack", ToolchainChannel::Rnd),
            "greentic-pack-rnd"
        );
        assert_eq!(
            delegated_binary_name_for_channel("greentic-runner-cli", ToolchainChannel::Rnd),
            "greentic-runner-cli-rnd"
        );
        assert_eq!(
            delegated_binary_name_for_channel("greentic-pack-rnd", ToolchainChannel::Rnd),
            "greentic-pack-rnd"
        );
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

    #[test]
    fn external_tool_env_key_is_plain_uppercase_no_channel_suffix() {
        // The key derives from the plain binary name; it must never carry a
        // `-dev`/`-rnd` channel suffix.
        assert_eq!(
            external_tool_env_key("greentic-mcp-gen"),
            "GREENTIC_DEV_BIN_GREENTIC_MCP_GEN"
        );
    }

    #[test]
    fn resolve_external_tool_errors_with_plain_name_when_absent() {
        // A name that is not on PATH and has no env override resolves to an error
        // that mentions the plain (unsuffixed) name.
        let err = resolve_external_tool("greentic-mcp-gen-absent-xyz")
            .expect_err("expected resolution to fail");
        assert!(err.to_string().contains("greentic-mcp-gen-absent-xyz"));
    }
}
