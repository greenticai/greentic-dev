mod confirm;
mod executor;
mod persistence;
pub mod plan;
mod provider;
mod registry;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::{WizardApplyArgs, WizardLaunchArgs, WizardValidateArgs};
use crate::i18n;
use crate::wizard::executor::ExecuteOptions;
use crate::wizard::plan::{WizardAnswers, WizardFrontend, WizardPlan};
use crate::wizard::provider::{ProviderRequest, ShellWizardProvider, WizardProvider};

const DEFAULT_LOCALE: &str = "en-US";
const DEFAULT_SCHEMA_VERSION: &str = "1.0.0";
const WIZARD_ID: &str = "greentic-dev.wizard.launcher.main";
const SCHEMA_ID: &str = "greentic-dev.launcher.main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    DryRun,
    Execute,
}

#[derive(Debug, Clone)]
struct LoadedAnswers {
    answers: serde_json::Value,
    inferred_locale: Option<String>,
    schema_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AnswerDocument {
    wizard_id: String,
    schema_id: String,
    schema_version: String,
    locale: String,
    answers: serde_json::Value,
    #[serde(default)]
    locks: BTreeMap<String, String>,
}

pub fn launch(args: WizardLaunchArgs) -> Result<()> {
    let mode = if args.dry_run {
        ExecutionMode::DryRun
    } else {
        ExecutionMode::Execute
    };

    let locale = i18n::select_locale(args.locale.as_deref());
    let answers = prompt_launcher_answers(mode, &locale)?;
    let loaded = LoadedAnswers {
        answers,
        inferred_locale: None,
        schema_version: args.schema_version.clone(),
    };

    run_from_inputs(
        args.frontend,
        Some(locale),
        loaded,
        args.out,
        mode,
        args.yes,
        args.non_interactive,
        args.unsafe_commands,
        args.allow_destructive,
        args.emit_answers,
        args.schema_version,
    )
}

pub fn validate(args: WizardValidateArgs) -> Result<()> {
    let loaded = load_answer_document(
        args.answers.as_path(),
        args.schema_version.as_deref(),
        args.migrate,
    )?;

    run_from_inputs(
        args.frontend,
        args.locale,
        loaded,
        args.out,
        ExecutionMode::DryRun,
        true,
        true,
        false,
        false,
        args.emit_answers,
        args.schema_version,
    )
}

pub fn apply(args: WizardApplyArgs) -> Result<()> {
    let loaded = load_answer_document(
        args.answers.as_path(),
        args.schema_version.as_deref(),
        args.migrate,
    )?;

    run_from_inputs(
        args.frontend,
        args.locale,
        loaded,
        args.out,
        ExecutionMode::Execute,
        args.yes,
        args.non_interactive,
        args.unsafe_commands,
        args.allow_destructive,
        args.emit_answers,
        args.schema_version,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_from_inputs(
    frontend_raw: String,
    cli_locale: Option<String>,
    loaded: LoadedAnswers,
    out: Option<PathBuf>,
    mode: ExecutionMode,
    yes: bool,
    non_interactive: bool,
    unsafe_commands: bool,
    allow_destructive: bool,
    emit_answers: Option<PathBuf>,
    requested_schema_version: Option<String>,
) -> Result<()> {
    let locale = i18n::select_locale(
        cli_locale
            .as_deref()
            .or(loaded.inferred_locale.as_deref())
            .or(Some(DEFAULT_LOCALE)),
    );
    let frontend = WizardFrontend::parse(&frontend_raw).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported frontend `{}`; expected text|json|adaptive-card",
            frontend_raw
        )
    })?;

    if registry::resolve("launcher", "main").is_none() {
        bail!("launcher mapping missing for `launcher.main`");
    }

    let merged_answers = merge_answers(None, None, Some(loaded.answers.clone()), None);
    let provider = ShellWizardProvider;
    let req = ProviderRequest {
        frontend: frontend.clone(),
        locale: locale.clone(),
        dry_run: mode == ExecutionMode::DryRun,
        answers: merged_answers.clone(),
    };
    let mut plan = provider.build_plan(&req)?;

    let out_dir = persistence::resolve_out_dir(out.as_deref());
    let paths = persistence::prepare_dir(&out_dir)?;
    persistence::persist_plan_and_answers(&paths, &merged_answers, &plan)?;

    render_plan(&plan)?;

    if mode == ExecutionMode::Execute {
        confirm::ensure_execute_allowed(
            &format!(
                "Plan `{}.{}` with {} step(s)",
                plan.metadata.target,
                plan.metadata.mode,
                plan.steps.len()
            ),
            yes,
            non_interactive,
        )?;
        let report = executor::execute(
            &plan,
            &paths.exec_log_path,
            &ExecuteOptions {
                unsafe_commands,
                allow_destructive,
            },
        )?;
        annotate_execution_metadata(&mut plan, &report);
        persistence::persist_plan_and_answers(&paths, &merged_answers, &plan)?;
    }

    if let Some(path) = emit_answers {
        let schema_version = requested_schema_version
            .or(loaded.schema_version)
            .unwrap_or_else(|| DEFAULT_SCHEMA_VERSION.to_string());
        let doc = build_answer_document(&locale, &schema_version, &merged_answers, &plan);
        write_answer_document(&path, &doc)?;
    }

    Ok(())
}

fn render_plan(plan: &WizardPlan) -> Result<()> {
    let rendered = match plan.metadata.frontend {
        WizardFrontend::Json => {
            serde_json::to_string_pretty(plan).context("failed to encode wizard plan")?
        }
        WizardFrontend::Text => render_text_plan(plan),
        WizardFrontend::AdaptiveCard => {
            let card = serde_json::json!({
                "type": "AdaptiveCard",
                "version": "1.5",
                "body": [
                    {"type":"TextBlock","weight":"Bolder","text":"greentic-dev launcher wizard plan"},
                    {"type":"TextBlock","text": "target: launcher mode: main"},
                ],
                "data": { "plan": plan }
            });
            serde_json::to_string_pretty(&card).context("failed to encode adaptive card")?
        }
    };
    println!("{rendered}");
    Ok(())
}

fn render_text_plan(plan: &WizardPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "wizard plan v{}: {}.{}\n",
        plan.plan_version, plan.metadata.target, plan.metadata.mode
    ));
    out.push_str(&format!("locale: {}\n", plan.metadata.locale));
    out.push_str(&format!("steps: {}\n", plan.steps.len()));
    for (idx, step) in plan.steps.iter().enumerate() {
        match step {
            crate::wizard::plan::WizardStep::RunCommand(cmd) => {
                out.push_str(&format!(
                    "{}. RunCommand {} {}\n",
                    idx + 1,
                    cmd.program,
                    cmd.args.join(" ")
                ));
            }
            other => out.push_str(&format!("{}. {:?}\n", idx + 1, other)),
        }
    }
    out
}

fn prompt_launcher_answers(mode: ExecutionMode, locale: &str) -> Result<serde_json::Value> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive {
        bail!(
            "{}",
            i18n::t(locale, "cli.wizard.error.interactive_required")
        );
    }

    eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.title"));
    eprintln!();
    eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.option_pack"));
    eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.option_bundle"));
    eprintln!();
    eprint!("{}", i18n::t(locale, "cli.wizard.launcher.select_option"));
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selected_action = match input.trim() {
        "1" => "pack",
        "2" => "bundle",
        _ => bail!("{}", i18n::t(locale, "cli.wizard.error.invalid_selection")),
    };

    let mut answers = serde_json::Map::new();
    answers.insert(
        "selected_action".to_string(),
        serde_json::Value::String(selected_action.to_string()),
    );
    if mode == ExecutionMode::DryRun {
        answers.insert(
            "delegate_answer_document".to_string(),
            serde_json::Value::Object(Default::default()),
        );
    }
    Ok(serde_json::Value::Object(answers))
}

fn load_answer_document(
    path: &Path,
    requested_schema_version: Option<&str>,
    migrate: bool,
) -> Result<LoadedAnswers> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    let mut doc: AnswerDocument = serde_json::from_value(value)
        .with_context(|| format!("failed to parse AnswerDocument from {}", path.display()))?;
    validate_answer_document_identity(&doc, path)?;

    if let Some(schema_version) = requested_schema_version
        && doc.schema_version != schema_version
    {
        if migrate {
            doc = migrate_answer_document(doc, schema_version);
        } else {
            bail!(
                "answers schema_version `{}` does not match requested `{}`; re-run with --migrate",
                doc.schema_version,
                schema_version
            );
        }
    }

    if !doc.answers.is_object() {
        bail!(
            "AnswerDocument `answers` must be a JSON object in {}",
            path.display()
        );
    }

    Ok(LoadedAnswers {
        answers: doc.answers.clone(),
        inferred_locale: Some(doc.locale),
        schema_version: Some(doc.schema_version),
    })
}

fn validate_answer_document_identity(doc: &AnswerDocument, path: &Path) -> Result<()> {
    if doc.wizard_id != WIZARD_ID {
        bail!(
            "unsupported wizard_id `{}` in {}; expected `{}`",
            doc.wizard_id,
            path.display(),
            WIZARD_ID
        );
    }
    if doc.schema_id != SCHEMA_ID {
        bail!(
            "unsupported schema_id `{}` in {}; expected `{}`",
            doc.schema_id,
            path.display(),
            SCHEMA_ID
        );
    }
    Ok(())
}

fn merge_answers(
    cli_overrides: Option<serde_json::Value>,
    parent_prefill: Option<serde_json::Value>,
    answers_file: Option<serde_json::Value>,
    provider_defaults: Option<serde_json::Value>,
) -> WizardAnswers {
    let mut out = BTreeMap::<String, serde_json::Value>::new();
    merge_obj(&mut out, provider_defaults);
    merge_obj(&mut out, answers_file);
    merge_obj(&mut out, parent_prefill);
    merge_obj(&mut out, cli_overrides);
    WizardAnswers {
        data: serde_json::Value::Object(out.into_iter().collect()),
    }
}

fn merge_obj(dst: &mut BTreeMap<String, serde_json::Value>, src: Option<serde_json::Value>) {
    if let Some(serde_json::Value::Object(map)) = src {
        for (k, v) in map {
            dst.insert(k, v);
        }
    }
}

fn migrate_answer_document(mut doc: AnswerDocument, target_schema_version: &str) -> AnswerDocument {
    doc.schema_version = target_schema_version.to_string();
    doc
}

fn build_answer_document(
    locale: &str,
    schema_version: &str,
    answers: &WizardAnswers,
    plan: &WizardPlan,
) -> AnswerDocument {
    AnswerDocument {
        wizard_id: WIZARD_ID.to_string(),
        schema_id: SCHEMA_ID.to_string(),
        schema_version: schema_version.to_string(),
        locale: locale.to_string(),
        answers: answers.data.clone(),
        locks: plan.inputs.clone(),
    }
}

fn write_answer_document(path: &Path, doc: &AnswerDocument) -> Result<()> {
    let rendered = serde_json::to_string_pretty(doc).context("render answers envelope JSON")?;
    fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
}

fn annotate_execution_metadata(
    plan: &mut WizardPlan,
    report: &crate::wizard::executor::ExecutionReport,
) {
    for (program, version) in &report.resolved_versions {
        plan.inputs
            .insert(format!("resolved_versions.{program}"), version.clone());
    }
    plan.inputs.insert(
        "executed_commands".to_string(),
        report.commands_executed.to_string(),
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde_json::json;

    use super::{
        AnswerDocument, SCHEMA_ID, WIZARD_ID, build_answer_document, merge_answers,
        validate_answer_document_identity,
    };
    use crate::wizard::plan::{WizardFrontend, WizardPlan, WizardPlanMetadata};

    #[test]
    fn answer_precedence_cli_over_file() {
        let merged = merge_answers(
            Some(json!({"foo":"cli"})),
            None,
            Some(json!({"foo":"file","bar":"file"})),
            None,
        );
        assert_eq!(merged.data["foo"], "cli");
        assert_eq!(merged.data["bar"], "file");
    }

    #[test]
    fn build_answer_document_sets_launcher_identity_fields() {
        let answers = merge_answers(None, None, Some(json!({"selected_action":"pack"})), None);
        let plan = WizardPlan {
            plan_version: 1,
            created_at: None,
            metadata: WizardPlanMetadata {
                target: "launcher".to_string(),
                mode: "main".to_string(),
                locale: "en-US".to_string(),
                frontend: WizardFrontend::Json,
            },
            inputs: BTreeMap::from([(
                "resolved_versions.greentic-pack".to_string(),
                "greentic-pack 0.1".to_string(),
            )]),
            steps: vec![],
        };

        let doc = build_answer_document("en-US", "1.0.0", &answers, &plan);

        assert_eq!(doc.wizard_id, WIZARD_ID);
        assert_eq!(doc.schema_id, SCHEMA_ID);
        assert_eq!(doc.schema_version, "1.0.0");
        assert_eq!(doc.locale, "en-US");
        assert_eq!(doc.answers["selected_action"], "pack");
        assert_eq!(
            doc.locks.get("resolved_versions.greentic-pack"),
            Some(&"greentic-pack 0.1".to_string())
        );
    }

    #[test]
    fn reject_non_launcher_answer_document_id() {
        let doc = AnswerDocument {
            wizard_id: "greentic-dev.wizard.pack.build".to_string(),
            schema_id: SCHEMA_ID.to_string(),
            schema_version: "1.0.0".to_string(),
            locale: "en-US".to_string(),
            answers: json!({}),
            locks: BTreeMap::new(),
        };
        let err = validate_answer_document_identity(&doc, Path::new("answers.json")).unwrap_err();
        assert!(err.to_string().contains("unsupported wizard_id"));
    }
}
