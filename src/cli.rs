use std::{ffi::OsString, path::PathBuf};

use crate::secrets_cli::SecretsCommand;
use clap::{Arg, ArgAction, Args, CommandFactory, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "greentic-dev")]
#[command(version)]
#[command(about = "cli.root.about")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

pub fn localized_help_command(locale: &str) -> clap::Command {
    let mut command = Cli::command()
        .about(crate::i18n::t(locale, "cli.root.about"))
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help(crate::i18n::t(locale, "cli.help.flag")),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help(crate::i18n::t(locale, "cli.version.flag")),
        )
        .arg(
            Arg::new("locale")
                .long("locale")
                .global(true)
                .value_name("LOCALE")
                .help(crate::i18n::t(locale, "cli.option.locale")),
        );

    for (name, key) in [
        ("flow", "cli.command.flow.about"),
        ("pack", "cli.command.pack.about"),
        ("component", "cli.command.component.about"),
        ("config", "cli.command.config.about"),
        ("mcp", "cli.command.mcp.about"),
        ("gui", "cli.command.gui.about"),
        ("secrets", "cli.command.secrets.about"),
        ("tools", "cli.command.tools.about"),
        ("install", "cli.command.install.about"),
        ("cbor", "cli.command.cbor.about"),
        ("wizard", "cli.command.wizard.about"),
    ] {
        command = command.mut_subcommand(name, |sub| sub.about(crate::i18n::t(locale, key)));
    }

    command = command.mut_subcommand("secrets", |sub| {
        sub.about(crate::i18n::t(locale, "cli.command.secrets.about"))
            .mut_subcommand("init", |sub| {
                sub.about(crate::i18n::t(locale, "cli.command.secrets.init.about"))
                    .mut_arg("pack", |arg| {
                        arg.help(crate::i18n::t(locale, "cli.command.secrets.init.pack"))
                    })
                    .mut_arg("passthrough", |arg| {
                        arg.help(crate::i18n::t(
                            locale,
                            "cli.command.secrets.init.passthrough",
                        ))
                    })
            })
    });
    command = command
        .mut_subcommand("config", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.config.about"))
                .mut_subcommand("set", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.config.set.about"))
                        .mut_arg("key", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.config.set.key"))
                        })
                        .mut_arg("value", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.config.set.value"))
                        })
                        .mut_arg("file", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.config.set.file"))
                        })
                })
        })
        .mut_subcommand("mcp", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.mcp.about"))
                .mut_subcommand("doctor", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.mcp.doctor.about"))
                        .mut_arg("provider", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.mcp.doctor.provider"))
                        })
                        .mut_arg("json", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.mcp.doctor.json"))
                        })
                })
        })
        .mut_subcommand("tools", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.tools.about"))
                .mut_subcommand("install", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.tools.install.about"))
                        .mut_arg("latest", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.tools.install.latest"))
                        })
                })
        })
        .mut_subcommand("install", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.install.about"))
                .mut_subcommand("tools", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.install.tools.about"))
                        .mut_arg("latest", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.tools.install.latest"))
                        })
                })
                .mut_arg("tenant", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.install.tenant"))
                })
                .mut_arg("token", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.install.token"))
                })
                .mut_arg("bin_dir", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.install.bin_dir"))
                })
                .mut_arg("docs_dir", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.install.docs_dir"))
                })
                .mut_arg("locale", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.install.locale"))
                })
        })
        .mut_subcommand("cbor", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.cbor.about"))
                .mut_arg("path", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.cbor.path"))
                })
        })
        .mut_subcommand("wizard", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.wizard.about"))
                .mut_arg("answers", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.answers"))
                })
                .mut_arg("frontend", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.frontend"))
                })
                .mut_arg("locale", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.locale"))
                })
                .mut_arg("emit_answers", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.emit_answers"))
                })
                .mut_arg("schema_version", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.schema_version"))
                })
                .mut_arg("migrate", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.migrate"))
                })
                .mut_arg("out", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.out"))
                })
                .mut_arg("dry_run", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.dry_run"))
                })
                .mut_arg("yes", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.yes"))
                })
                .mut_arg("non_interactive", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.non_interactive"))
                })
                .mut_arg("unsafe_commands", |arg| {
                    arg.help(crate::i18n::t(locale, "cli.command.wizard.unsafe_commands"))
                })
                .mut_arg("allow_destructive", |arg| {
                    arg.help(crate::i18n::t(
                        locale,
                        "cli.command.wizard.allow_destructive",
                    ))
                })
                .mut_subcommand("validate", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.wizard.validate.about"))
                        .mut_arg("answers", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.answers"))
                        })
                        .mut_arg("frontend", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.frontend"))
                        })
                        .mut_arg("locale", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.locale"))
                        })
                        .mut_arg("emit_answers", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.emit_answers"))
                        })
                        .mut_arg("schema_version", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.schema_version"))
                        })
                        .mut_arg("migrate", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.migrate"))
                        })
                        .mut_arg("out", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.out"))
                        })
                })
                .mut_subcommand("apply", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.wizard.apply.about"))
                        .mut_arg("answers", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.answers"))
                        })
                        .mut_arg("frontend", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.frontend"))
                        })
                        .mut_arg("locale", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.locale"))
                        })
                        .mut_arg("emit_answers", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.emit_answers"))
                        })
                        .mut_arg("schema_version", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.schema_version"))
                        })
                        .mut_arg("migrate", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.migrate"))
                        })
                        .mut_arg("out", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.out"))
                        })
                        .mut_arg("yes", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.yes"))
                        })
                        .mut_arg("non_interactive", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.non_interactive"))
                        })
                        .mut_arg("unsafe_commands", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.wizard.unsafe_commands"))
                        })
                        .mut_arg("allow_destructive", |arg| {
                            arg.help(crate::i18n::t(
                                locale,
                                "cli.command.wizard.allow_destructive",
                            ))
                        })
                })
        });

    localize_help_tree(command, locale, true)
}

fn localize_help_tree(mut command: clap::Command, locale: &str, is_root: bool) -> clap::Command {
    command = command
        .disable_help_subcommand(true)
        .disable_help_flag(true);
    let arg_ids = command
        .get_arguments()
        .map(|arg| arg.get_id().as_str().to_string())
        .collect::<Vec<_>>();
    if arg_ids.iter().any(|id| id == "help") {
        command = command.mut_arg("help", |arg| {
            arg.help(crate::i18n::t(locale, "cli.help.flag"))
        });
    } else {
        command = command.arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help(crate::i18n::t(locale, "cli.help.flag")),
        );
    }
    if is_root && arg_ids.iter().any(|id| id == "version") {
        command = command.mut_arg("version", |arg| {
            arg.help(crate::i18n::t(locale, "cli.version.flag"))
        });
    }

    let sub_names = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .collect::<Vec<_>>();
    for name in sub_names {
        command = command.mut_subcommand(name, |sub| localize_help_tree(sub, locale, false));
    }

    command
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// cli.command.flow.about
    Flow(PassthroughArgs),
    /// cli.command.pack.about
    Pack(PassthroughArgs),
    /// cli.command.component.about
    Component(PassthroughArgs),
    /// cli.command.config.about
    #[command(subcommand)]
    Config(ConfigCommand),
    /// cli.command.mcp.about
    #[command(subcommand)]
    Mcp(McpCommand),
    /// cli.command.gui.about
    Gui(PassthroughArgs),
    /// cli.command.secrets.about
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// cli.command.tools.about
    #[command(subcommand)]
    Tools(ToolsCommand),
    /// cli.command.install.about
    Install(InstallArgs),
    /// cli.command.cbor.about
    Cbor(CborArgs),
    /// cli.command.wizard.about
    Wizard(Box<WizardCommand>),
}

#[derive(Args, Debug, Clone)]
#[command(disable_help_flag = true)]
pub struct PassthroughArgs {
    /// cli.command.passthrough.args
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<OsString>,
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// cli.command.mcp.doctor.about
    Doctor(McpDoctorArgs),
}

#[derive(Args, Debug)]
pub struct McpDoctorArgs {
    /// cli.command.mcp.doctor.provider
    pub provider: String,
    /// cli.command.mcp.doctor.json
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// cli.command.config.set.about
    Set(ConfigSetArgs),
}

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// cli.command.tools.install.about
    Install(ToolsInstallArgs),
}

#[derive(Subcommand, Debug)]
pub enum InstallSubcommand {
    /// cli.command.install.tools.about
    Tools(ToolsInstallArgs),
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[command(subcommand)]
    pub command: Option<InstallSubcommand>,
    /// cli.command.install.tenant
    #[arg(long = "tenant")]
    pub tenant: Option<String>,
    /// cli.command.install.token
    #[arg(long = "token")]
    pub token: Option<String>,
    /// cli.command.install.bin_dir
    #[arg(long = "bin-dir")]
    pub bin_dir: Option<PathBuf>,
    /// cli.command.install.docs_dir
    #[arg(long = "docs-dir")]
    pub docs_dir: Option<PathBuf>,
    /// cli.command.install.locale
    #[arg(long = "locale")]
    pub locale: Option<String>,
}

#[derive(Args, Debug)]
pub struct ToolsInstallArgs {
    /// cli.command.tools.install.latest
    #[arg(long = "latest")]
    pub latest: bool,
}

#[derive(Args, Debug)]
pub struct ConfigSetArgs {
    /// cli.command.config.set.key
    pub key: String,
    /// cli.command.config.set.value
    pub value: String,
    /// cli.command.config.set.file
    #[arg(long = "file")]
    pub file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct CborArgs {
    /// cli.command.cbor.path
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
    /// cli.command.wizard.validate.about
    Validate(WizardValidateArgs),
    /// cli.command.wizard.apply.about
    Apply(WizardApplyArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WizardLaunchArgs {
    /// cli.command.wizard.answers (local path or http/https URL)
    #[arg(long = "answers")]
    pub answers: Option<String>,
    /// cli.command.wizard.frontend
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// cli.command.wizard.locale
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// cli.command.wizard.emit_answers
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// cli.command.wizard.schema_version
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// cli.command.wizard.migrate
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// cli.command.wizard.out
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
    /// cli.command.wizard.dry_run
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// cli.command.wizard.yes
    #[arg(long = "yes")]
    pub yes: bool,
    /// cli.command.wizard.non_interactive
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// cli.command.wizard.unsafe_commands
    #[arg(long = "unsafe-commands")]
    pub unsafe_commands: bool,
    /// cli.command.wizard.allow_destructive
    #[arg(long = "allow-destructive")]
    pub allow_destructive: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WizardValidateArgs {
    /// cli.command.wizard.answers (local path or http/https URL)
    #[arg(long = "answers")]
    pub answers: String,
    /// cli.command.wizard.frontend
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// cli.command.wizard.locale
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// cli.command.wizard.emit_answers
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// cli.command.wizard.schema_version
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// cli.command.wizard.migrate
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// cli.command.wizard.out
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct WizardApplyArgs {
    /// cli.command.wizard.answers (local path or http/https URL)
    #[arg(long = "answers")]
    pub answers: String,
    /// cli.command.wizard.frontend
    #[arg(long = "frontend", default_value = "json")]
    pub frontend: String,
    /// cli.command.wizard.locale
    #[arg(long = "locale")]
    pub locale: Option<String>,
    /// cli.command.wizard.emit_answers
    #[arg(long = "emit-answers")]
    pub emit_answers: Option<PathBuf>,
    /// cli.command.wizard.schema_version
    #[arg(long = "schema-version")]
    pub schema_version: Option<String>,
    /// cli.command.wizard.migrate
    #[arg(long = "migrate")]
    pub migrate: bool,
    /// cli.command.wizard.out
    #[arg(long = "out")]
    pub out: Option<PathBuf>,
    /// cli.command.wizard.yes
    #[arg(long = "yes")]
    pub yes: bool,
    /// cli.command.wizard.non_interactive
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// cli.command.wizard.unsafe_commands
    #[arg(long = "unsafe-commands")]
    pub unsafe_commands: bool,
    /// cli.command.wizard.allow_destructive
    #[arg(long = "allow-destructive")]
    pub allow_destructive: bool,
}
