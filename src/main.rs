use anyhow::Result;
use clap::Parser;
use clap::error::ErrorKind;
use std::env;
use std::ffi::OsString;
use std::process::{Command as ProcessCommand, Stdio};

use greentic_dev::cli::{Cli, Command};
use greentic_dev::cli::{
    InstallSubcommand, McpCommand, ReleaseCommand, ToolsCommand, WizardSubcommand,
};
use greentic_dev::dw_cmd;
use greentic_dev::passthrough::{delegated_binary_name, resolve_binary, run_passthrough};

use greentic_dev::cbor_cmd;
use greentic_dev::cmd::config;
use greentic_dev::cmd::tools;
use greentic_dev::coverage_cmd;
use greentic_dev::install;
use greentic_dev::mcp_cmd;
use greentic_dev::release_cmd;
use greentic_dev::secrets_cli::run_secrets_command;
use greentic_dev::security_cmd;
use greentic_dev::wizard;

fn main() -> Result<()> {
    let argv: Vec<OsString> = env::args_os().collect();
    maybe_delegate_external_subcommand(&argv);
    maybe_delegate_mcp_passthrough(&argv)?;
    maybe_render_localized_help(&argv);
    let selected_locale = greentic_dev::i18n::select_locale(
        greentic_dev::i18n::cli_locale_from_argv(&argv).as_deref(),
    );

    let cli = Cli::parse();

    match cli.command {
        Command::Flow(args) => {
            let bin = resolve_binary("greentic-flow")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Pack(args) => {
            let subcommand = args.args.first().and_then(|s| s.to_str());
            if subcommand == Some("run") {
                let bin = resolve_binary("greentic-runner-cli")?;
                let run_args = &args.args[1..];
                let status = run_passthrough(&bin, run_args, false)?;
                std::process::exit(status.code().unwrap_or(1));
            }

            let bin = resolve_binary("greentic-pack")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Component(args) => {
            let bin = resolve_binary("greentic-component")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Bundle(args) => {
            let bin = resolve_binary("greentic-bundle")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Runner(args) => {
            let bin = resolve_binary("greentic-runner")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Config(config_cmd) => config::run(config_cmd),
        Command::Coverage(args) => coverage_cmd::run(args),
        Command::Security(args) => {
            std::process::exit(security_cmd::run(args));
        }
        Command::Cbor(args) => cbor_cmd::run(args),
        Command::Mcp(mcp) => match mcp {
            McpCommand::Doctor(args) => mcp_cmd::doctor(args.provider.as_deref(), args.json),
            McpCommand::Gen(args) => {
                let bin = greentic_dev::passthrough::resolve_external_tool("greentic-mcp-gen")
                    .map_err(|_| {
                        anyhow::anyhow!(greentic_dev::i18n::t(
                            &selected_locale,
                            "runtime.mcp.gen.error.not_installed"
                        ))
                    })?;
                let status = run_passthrough(&bin, &args.args, false)?;
                std::process::exit(status.code().unwrap_or(1));
            }
        },
        Command::Tools(command) => match command {
            ToolsCommand::Install(args) => tools::install(args.latest, &selected_locale),
        },
        Command::Install(args) => {
            let install_locale = args
                .locale
                .clone()
                .unwrap_or_else(|| selected_locale.clone());
            match args.command {
                Some(InstallSubcommand::Tools(args)) => {
                    tools::install(args.latest, &install_locale)
                }
                None => install::run(args),
            }
        }
        Command::Release(release) => match release {
            ReleaseCommand::Generate(args) => release_cmd::generate(args),
            ReleaseCommand::Publish(args) => release_cmd::publish(args),
            ReleaseCommand::View(args) => release_cmd::view(args),
            ReleaseCommand::Latest(args) => release_cmd::latest(args),
            ReleaseCommand::Promote(args) => release_cmd::promote(args),
            ReleaseCommand::Snapshot(args) => release_cmd::snapshot(args),
        },
        Command::Wizard(args) => match args.command {
            Some(WizardSubcommand::Validate(sub)) => wizard::validate(sub),
            Some(WizardSubcommand::Apply(sub)) => wizard::apply(sub),
            None => wizard::launch(args.launch),
        },
        Command::Gui(args) => {
            let bin = resolve_binary("greentic-gui")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Secrets(secrets) => run_secrets_command(secrets, &selected_locale),
        Command::Dw(dw) => dw_cmd::run(dw, &selected_locale),
    }
}

fn maybe_delegate_mcp_passthrough(argv: &[OsString]) -> Result<()> {
    let Some(raw_subcmd) = argv.get(1) else {
        return Ok(());
    };
    let Some(subcmd) = raw_subcmd.to_str() else {
        return Ok(());
    };
    if subcmd != "mcp" {
        return Ok(());
    }

    let Some(raw_mcp_arg) = argv.get(2) else {
        return Ok(());
    };
    let Some(mcp_arg) = raw_mcp_arg.to_str() else {
        return Ok(());
    };

    if matches!(mcp_arg, "doctor" | "-h" | "--help") {
        return Ok(());
    }

    if mcp_arg == "gen" {
        let locale = greentic_dev::i18n::select_locale(
            greentic_dev::i18n::cli_locale_from_argv(argv).as_deref(),
        );
        // Safe: `mcp_gen_args` returns Some here because argv[1]=="mcp" && argv[2]=="gen".
        let forwarded = mcp_gen_args(argv).unwrap_or_default();
        let bin =
            greentic_dev::passthrough::resolve_external_tool("greentic-mcp-gen").map_err(|_| {
                anyhow::anyhow!(greentic_dev::i18n::t(
                    &locale,
                    "runtime.mcp.gen.error.not_installed"
                ))
            })?;
        let status = run_passthrough(&bin, &forwarded, false)?;
        std::process::exit(status.code().unwrap_or(1));
    }

    let bin = resolve_binary("greentic-mcp")?;
    let delegated_args = rewritten_mcp_passthrough_args(&argv[2..]);
    let status = run_passthrough(&bin, &delegated_args, false)?;
    std::process::exit(status.code().unwrap_or(1));
}

/// If `argv` is `greentic-dev mcp gen …`, return the tokens after `gen`
/// (everything to forward to the generator). Otherwise `None`.
fn mcp_gen_args(argv: &[OsString]) -> Option<Vec<OsString>> {
    if argv.get(1)?.to_str()? != "mcp" {
        return None;
    }
    if argv.get(2)?.to_str()? != "gen" {
        return None;
    }
    Some(argv[3..].to_vec())
}

fn rewritten_mcp_passthrough_args(args: &[OsString]) -> Vec<OsString> {
    if let Some(first) = args.first()
        && first.to_str() == Some("--compose")
    {
        let mut rewritten = Vec::with_capacity(args.len());
        rewritten.push(OsString::from("compose"));
        rewritten.extend(args[1..].iter().cloned());
        return rewritten;
    }

    args.to_vec()
}

fn maybe_render_localized_help(argv: &[OsString]) {
    let wants_help = argv
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")));
    if !wants_help {
        return;
    }

    let help_path = help_subcommand_path(argv);
    let first_command = help_path.first().map(String::as_str);

    if matches!(
        first_command,
        Some("flow" | "pack" | "component" | "bundle" | "runner" | "gui")
    ) {
        return;
    }

    let locale = greentic_dev::i18n::select_locale(
        greentic_dev::i18n::cli_locale_from_argv(argv).as_deref(),
    );
    let mut command = greentic_dev::cli::localized_help_command(&locale);
    match first_command {
        None => {
            let _ = command.print_long_help();
            println!();
            std::process::exit(0);
        }
        Some(name) if is_known_subcommand(name) && name != "help" => {
            if print_subcommand_help(&mut command, &help_path) {
                println!();
                std::process::exit(0);
            }
        }
        _ => {
            if let Err(err) = command.try_get_matches_from_mut(argv.iter().cloned())
                && matches!(
                    err.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                )
            {
                let _ = err.print();
                std::process::exit(0);
            }
        }
    }
}

fn help_subcommand_path(argv: &[OsString]) -> Vec<String> {
    let mut path = Vec::new();
    let mut skip_next = false;
    for arg in argv.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        let Some(text) = arg.to_str() else {
            continue;
        };
        match text {
            "-h" | "--help" => break,
            "--locale" => {
                skip_next = true;
            }
            _ if text.starts_with("--locale=") => {}
            _ if text.starts_with('-') => {}
            _ => path.push(text.to_string()),
        }
    }
    path
}

fn print_subcommand_help(command: &mut clap::Command, path: &[String]) -> bool {
    if let Some((segment, rest)) = path.split_first()
        && let Some(next) = command.find_subcommand_mut(segment)
    {
        if rest.is_empty() {
            let _ = next.print_long_help();
            return true;
        }
        return print_subcommand_help(next, rest);
    }
    false
}

fn maybe_delegate_external_subcommand(argv: &[OsString]) {
    let Some(raw_subcmd) = argv.get(1) else {
        return;
    };

    let Some(subcmd) = raw_subcmd.to_str() else {
        return;
    };

    if subcmd.starts_with('-') || is_known_subcommand(subcmd) {
        return;
    }

    try_delegate_to_prefixed(subcmd, &argv[2..]);
}

fn is_known_subcommand(subcmd: &str) -> bool {
    matches!(
        subcmd,
        "flow"
            | "pack"
            | "component"
            | "bundle"
            | "runner"
            | "config"
            | "coverage"
            | "mcp"
            | "gui"
            | "secrets"
            | "dw"
            | "tools"
            | "install"
            | "release"
            | "cbor"
            | "wizard"
            | "help"
    )
}

fn try_delegate_to_prefixed(subcmd: &str, rest: &[OsString]) {
    let exe = delegated_binary_name(&format!("greentic-{subcmd}"));

    let status = match ProcessCommand::new(&exe)
        .args(rest)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            let locale = greentic_dev::i18n::select_locale(None);
            eprintln!(
                "{}",
                greentic_dev::i18n::tf(
                    &locale,
                    "runtime.main.error.execute_external",
                    &[("exe", exe), ("error", err.to_string())],
                )
            );
            std::process::exit(127);
        }
    };

    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn argv(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn mcp_gen_args_captures_everything_after_gen() {
        let a = argv(&[
            "greentic-dev",
            "mcp",
            "gen",
            "--spec",
            "./api.yaml",
            "--output-dir",
            "./out",
        ]);
        let forwarded = mcp_gen_args(&a).expect("mcp gen should be recognized");
        assert_eq!(
            forwarded,
            argv(&["--spec", "./api.yaml", "--output-dir", "./out"])
        );
    }

    #[test]
    fn mcp_gen_args_forwards_subcommand_style_args_verbatim() {
        let a = argv(&[
            "greentic-dev",
            "mcp",
            "gen",
            "discovery",
            "--url",
            "https://x/y",
            "--dry-run",
        ]);
        let forwarded = mcp_gen_args(&a).expect("mcp gen should be recognized");
        assert_eq!(
            forwarded,
            argv(&["discovery", "--url", "https://x/y", "--dry-run"])
        );
    }

    #[test]
    fn mcp_gen_args_ignores_non_gen() {
        assert!(mcp_gen_args(&argv(&["greentic-dev", "mcp", "doctor", "providers"])).is_none());
        assert!(mcp_gen_args(&argv(&["greentic-dev", "flow", "gen"])).is_none());
        assert!(mcp_gen_args(&argv(&["greentic-dev", "mcp"])).is_none());
    }
}
