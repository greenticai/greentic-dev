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
    Install(InstallArgs),
    /// Decode a CBOR file to text
    Cbor(CborArgs),
    /// Deterministic orchestration for dev workbench workflows
    Wizard(Box<WizardCommand>),
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
pub enum InstallSubcommand {
    /// Install delegated tools (component/flow/pack/gui/runner/secrets)
    Tools(ToolsInstallArgs),
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[command(subcommand)]
    pub command: Option<InstallSubcommand>,
    /// Tenant identifier for commercial installs
    #[arg(long = "tenant")]
    pub tenant: Option<String>,
    /// Auth token or env:VAR indirection for commercial installs
    #[arg(long = "token")]
    pub token: Option<String>,
    /// Override the directory used for installed binaries
    #[arg(long = "bin-dir")]
    pub bin_dir: Option<PathBuf>,
    /// Override the directory used for installed docs
    #[arg(long = "docs-dir")]
    pub docs_dir: Option<PathBuf>,
    /// Locale (BCP47) used for translated install manifests/docs
    #[arg(long = "locale")]
    pub locale: Option<String>,
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

#[derive(Args, Debug, Clone)]
pub struct WizardCommand {
    #[command(subcommand)]
    pub command: Option<WizardSubcommand>,
    #[command(flatten)]
    pub launch: WizardLaunchArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub enum WizardSubcommand {
    /// Validate a launcher AnswerDocument non-interactively
    Validate(WizardValidateArgs),
    /// Apply a launcher AnswerDocument non-interactively
    Apply(WizardApplyArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WizardLaunchArgs {
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
    /// Preview only (default mode is apply when --dry-run is not set)
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
}

#[derive(Args, Debug, Clone)]
pub struct WizardValidateArgs {
    /// Answers file (AnswerDocument envelope)
    #[arg(long = "answers")]
    pub answers: PathBuf,
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
    /// Answers file (AnswerDocument envelope)
    #[arg(long = "answers")]
    pub answers: PathBuf,
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
