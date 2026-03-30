use anyhow::Result;
use clap::Parser;
use clap::error::ErrorKind;
use std::env;
use std::ffi::OsString;
use std::process::{Command as ProcessCommand, Stdio};

use greentic_dev::cli::{Cli, Command};
use greentic_dev::cli::{InstallSubcommand, McpCommand, ToolsCommand, WizardSubcommand};
use greentic_dev::passthrough::{resolve_binary, run_passthrough};

use greentic_dev::cbor_cmd;
use greentic_dev::cmd::config;
use greentic_dev::cmd::tools;
use greentic_dev::coverage_cmd;
use greentic_dev::install;
use greentic_dev::mcp_cmd;
use greentic_dev::secrets_cli::run_secrets_command;
use greentic_dev::wizard;

fn main() -> Result<()> {
    let argv: Vec<OsString> = env::args_os().collect();
    maybe_delegate_external_subcommand(&argv);
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
        Command::Config(config_cmd) => config::run(config_cmd),
        Command::Coverage(args) => coverage_cmd::run(args),
        Command::Cbor(args) => cbor_cmd::run(args),
        Command::Mcp(mcp) => match mcp {
            McpCommand::Doctor(args) => mcp_cmd::doctor(&args.provider, args.json),
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
    }
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

    if matches!(first_command, Some("flow" | "pack" | "component" | "gui")) {
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
            | "config"
            | "coverage"
            | "mcp"
            | "gui"
            | "secrets"
            | "tools"
            | "install"
            | "cbor"
            | "wizard"
            | "help"
    )
}

fn try_delegate_to_prefixed(subcmd: &str, rest: &[OsString]) {
    let exe = format!("greentic-{subcmd}");

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
