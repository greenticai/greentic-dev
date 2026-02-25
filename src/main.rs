use anyhow::Result;
use clap::Parser;
use std::env;
use std::ffi::OsString;
use std::process::{Command as ProcessCommand, Stdio};

use greentic_dev::cli::{Cli, Command};
use greentic_dev::cli::{InstallCommand, McpCommand, ToolsCommand, WizardCommand};
use greentic_dev::passthrough::{resolve_binary, run_passthrough};

use greentic_dev::cbor_cmd;
use greentic_dev::cmd::config;
use greentic_dev::cmd::tools;
use greentic_dev::mcp_cmd;
use greentic_dev::secrets_cli::run_secrets_command;
use greentic_dev::wizard;

fn main() -> Result<()> {
    let argv: Vec<OsString> = env::args_os().collect();
    maybe_delegate_external_subcommand(&argv);

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
        Command::Cbor(args) => cbor_cmd::run(args),
        Command::Mcp(mcp) => match mcp {
            McpCommand::Doctor(args) => mcp_cmd::doctor(&args.provider, args.json),
        },
        Command::Tools(command) => match command {
            ToolsCommand::Install(args) => tools::install(args.latest),
        },
        Command::Install(command) => match command {
            InstallCommand::Tools(args) => tools::install(args.latest),
        },
        Command::Wizard(command) => match command {
            WizardCommand::Run(args) => wizard::run(args),
            WizardCommand::Replay(args) => wizard::replay(args),
        },
        Command::Gui(args) => {
            let bin = resolve_binary("greentic-gui")?;
            let status = run_passthrough(&bin, &args.args, false)?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Secrets(secrets) => run_secrets_command(secrets),
    }
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
            eprintln!("Failed to execute {exe}: {err}");
            std::process::exit(127);
        }
    };

    std::process::exit(status.code().unwrap_or(1));
}
