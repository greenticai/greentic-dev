use std::{ffi::OsString, path::PathBuf};

use crate::secrets_cli::SecretsCommand;
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "greentic-dev")]
#[command(version)]
#[command(about = "Greentic developer tooling CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Flow passthrough (greentic-flow)
    Flow(PassthroughArgs),
    /// Pack passthrough (greentic-pack; pack run uses greentic-runner-cli)
    Pack(PassthroughArgs),
    /// Component passthrough (greentic-component)
    Component(PassthroughArgs),
    /// Manage greentic-dev configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// MCP tooling
    #[command(subcommand)]
    Mcp(McpCommand),
    /// GUI passthrough (greentic-gui)
    Gui(PassthroughArgs),
    /// Secrets convenience wrappers
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// Install/update delegated Greentic tool binaries
    #[command(subcommand)]
    Tools(ToolsCommand),
    /// Install delegated assets
    #[command(subcommand)]
    Install(InstallCommand),
    /// Decode a CBOR file to text
    Cbor(CborArgs),
    /// Deterministic orchestration for dev workbench workflows
    #[command(subcommand)]
    Wizard(WizardCommand),
}

#[derive(Args, Debug, Clone)]
#[command(disable_help_flag = true)]
pub struct PassthroughArgs {
    /// Arguments passed directly to the underlying command
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<OsString>,
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// Inspect MCP provider metadata
    Doctor(McpDoctorArgs),
}

#[derive(Args, Debug)]
pub struct McpDoctorArgs {
    /// MCP provider identifier or config path
    pub provider: String,
    /// Emit compact JSON instead of pretty output
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Set a key in greentic-dev config (e.g. defaults.component.org)
    Set(ConfigSetArgs),
}

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// Install delegated tools (component/flow/pack/gui/runner/secrets)
    Install(ToolsInstallArgs),
}

#[derive(Subcommand, Debug)]
pub enum InstallCommand {
    /// Install delegated tools (component/flow/pack/gui/runner/secrets)
    Tools(ToolsInstallArgs),
}

#[derive(Args, Debug)]
pub struct ToolsInstallArgs {
    /// Reinstall tools to pull latest available versions
    #[arg(long = "latest")]
    pub latest: bool,
}

#[derive(Args, Debug)]
pub struct ConfigSetArgs {
    /// Config key path (e.g. defaults.component.org)
    pub key: String,
    /// Value to assign to the key (stored as a string)
    pub value: String,
    /// Override config file path (default: $XDG_CONFIG_HOME/greentic-dev/config.toml)
    #[arg(long = "file")]
    pub file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct CborArgs {
    /// Path to the CBOR file to decode
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum WizardCommand {
    /// Build a deterministic wizard plan and optionally execute it
    Run(WizardRunArgs),
    /// Validate a wizard plan non-interactively from answers input
    Validate(WizardValidateArgs),
    /// Apply a wizard plan non-interactively from answers input
    Apply(WizardApplyArgs),
    /// Replay a previously persisted wizard plan + answers
    Replay(WizardReplayArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WizardRunArgs {
    /// Target domain
    #[arg(long = "target")]
    pub target: String,
    /// Operation mode for the target
    #[arg(long = "mode")]
    pub mode: String,
    /// Frontend mode (text/json/adaptive-card)
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// Locale (BCP47), passed to providers and recorded in plan metadata
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// Answers file (JSON object)
    #[arg(long = "answers")]
    pub answers: Option<PathBuf>,
    /// Emit a portable AnswerDocument envelope JSON file
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// Pin schema version for emitted/validated AnswerDocument
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// Migrate AnswerDocument to the selected schema version when needed
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// Override output directory (default: `.greentic/wizard/<run-id>/`)
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
    /// Preview only (default when neither --dry-run nor --execute is set)
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Execute plan steps
    #[arg(long = "execute")]
    pub execute: bool,
    /// Skip interactive confirmation prompt
    #[arg(long = "yes")]
    pub yes: bool,
    /// Allow execution in non-interactive contexts
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// Allow commands outside the default run-command allowlist
    #[arg(long = "unsafe-commands")]
    pub unsafe_commands: bool,
    /// Allow destructive operations (delete/overwrite/move) when requested by a plan step
    #[arg(long = "allow-destructive")]
    pub allow_destructive: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WizardValidateArgs {
    /// Answers file (AnswerDocument envelope or legacy answers JSON object)
    #[arg(long = "answers")]
    pub answers: PathBuf,
    /// Target domain (optional when inferrable from AnswerDocument wizard_id)
    #[arg(long = "target")]
    pub target: Option<String>,
    /// Operation mode (optional when inferrable from AnswerDocument wizard_id)
    #[arg(long = "mode")]
    pub mode: Option<String>,
    /// Frontend mode (text/json/adaptive-card)
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// Locale (BCP47), passed to providers and recorded in plan metadata
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// Emit a portable AnswerDocument envelope JSON file
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// Pin schema version for emitted/validated AnswerDocument
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// Migrate AnswerDocument to the selected schema version when needed
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// Override output directory (default: `.greentic/wizard/<run-id>/`)
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct WizardApplyArgs {
    /// Answers file (AnswerDocument envelope or legacy answers JSON object)
    #[arg(long = "answers")]
    pub answers: PathBuf,
    /// Target domain (optional when inferrable from AnswerDocument wizard_id)
    #[arg(long = "target")]
    pub target: Option<String>,
    /// Operation mode (optional when inferrable from AnswerDocument wizard_id)
    #[arg(long = "mode")]
    pub mode: Option<String>,
    /// Frontend mode (text/json/adaptive-card)
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// Locale (BCP47), passed to providers and recorded in plan metadata
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// Emit a portable AnswerDocument envelope JSON file
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// Pin schema version for emitted/validated AnswerDocument
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// Migrate AnswerDocument to the selected schema version when needed
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// Override output directory (default: `.greentic/wizard/<run-id>/`)
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
    /// Skip interactive confirmation prompt
    #[arg(long = "yes")]
    pub yes: bool,
    /// Allow execution in non-interactive contexts
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// Allow commands outside the default run-command allowlist
    #[arg(long = "unsafe-commands")]
    pub unsafe_commands: bool,
    /// Allow destructive operations (delete/overwrite/move) when requested by a plan step
    #[arg(long = "allow-destructive")]
    pub allow_destructive: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WizardReplayArgs {
    /// Path to a persisted answers file from a prior run
    #[arg(long = "answers")]
    pub answers: PathBuf,
    /// Execute plan steps
    #[arg(long = "execute")]
    pub execute: bool,
    /// Preview only (default when neither --dry-run nor --execute is set)
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Skip interactive confirmation prompt
    #[arg(long = "yes")]
    pub yes: bool,
    /// Allow execution in non-interactive contexts
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// Allow commands outside the default run-command allowlist
    #[arg(long = "unsafe-commands")]
    pub unsafe_commands: bool,
    /// Allow destructive operations (delete/overwrite/move) when requested by a plan step
    #[arg(long = "allow-destructive")]
    pub allow_destructive: bool,
    /// Override output directory (default: reuse answers parent)
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
}
