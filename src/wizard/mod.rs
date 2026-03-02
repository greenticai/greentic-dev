mod confirm;
mod executor;
mod persistence;
pub mod plan;
mod provider;
mod registry;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::{WizardApplyArgs, WizardReplayArgs, WizardRunArgs, WizardValidateArgs};
use crate::wizard::executor::ExecuteOptions;
use crate::wizard::plan::{WizardAnswers, WizardFrontend, WizardPlan};
use crate::wizard::provider::{ProviderRequest, ShellWizardProvider, WizardProvider};

const DEFAULT_LOCALE: &str = "en-US";
const DEFAULT_SCHEMA_VERSION: &str = "1.0.0";
const WIZARD_ID_PREFIX: &str = "greentic-dev.wizard.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    DryRun,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetMode {
    target: String,
    mode: String,
}

#[derive(Debug, Clone)]
struct LoadedAnswers {
    answers: serde_json::Value,
    inferred_target_mode: Option<TargetMode>,
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

pub fn run(args: WizardRunArgs) -> Result<()> {
    let mode = resolve_execution_mode(args.dry_run, args.execute)?;
    let loaded = load_answers_input(
        args.answers.as_deref(),
        args.schema_version.as_deref(),
        args.migrate,
    )?;
    let target_mode = TargetMode {
        target: args.target,
        mode: args.mode,
    };
    run_from_inputs(
        target_mode,
        args.frontend,
        args.locale,
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
    let loaded = load_answers_input(
        Some(args.answers.as_path()),
        args.schema_version.as_deref(),
        args.migrate,
    )?;
    let target_mode =
        resolve_target_mode(args.target, args.mode, loaded.inferred_target_mode.clone())?;

    run_from_inputs(
        target_mode,
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
    let loaded = load_answers_input(
        Some(args.answers.as_path()),
        args.schema_version.as_deref(),
        args.migrate,
    )?;
    let target_mode =
        resolve_target_mode(args.target, args.mode, loaded.inferred_target_mode.clone())?;

    run_from_inputs(
        target_mode,
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

pub fn replay(args: WizardReplayArgs) -> Result<()> {
    let mode = resolve_execution_mode(args.dry_run, args.execute)?;
    let (answers, mut plan, replay_root) = persistence::load_replay(&args.answers)?;
    let out_dir = args.out.unwrap_or(replay_root);
    let paths = persistence::prepare_dir(&out_dir)?;
    persistence::persist_plan_and_answers(&paths, &answers, &plan)?;
    render_plan(&plan)?;

    if mode == ExecutionMode::Execute {
        confirm::ensure_execute_allowed(
            &format!(
                "Replay plan `{}.{}` with {} step(s)",
                plan.metadata.target,
                plan.metadata.mode,
                plan.steps.len()
            ),
            args.yes,
            args.non_interactive,
        )?;
        let report = executor::execute(
            &plan,
            &paths.exec_log_path,
            &ExecuteOptions {
                unsafe_commands: args.unsafe_commands,
                allow_destructive: args.allow_destructive,
            },
        )?;
        annotate_execution_metadata(&mut plan, &report);
        persistence::persist_plan_and_answers(&paths, &answers, &plan)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_from_inputs(
    target_mode: TargetMode,
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
    let locale = cli_locale
        .or(loaded.inferred_locale.clone())
        .unwrap_or_else(|| DEFAULT_LOCALE.to_string());
    let frontend = WizardFrontend::parse(&frontend_raw).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported frontend `{}`; expected text|json|adaptive-card",
            frontend_raw
        )
    })?;

    if registry::resolve(&target_mode.target, &target_mode.mode).is_none() {
        bail!(
            "unsupported wizard target/mode `{}.{}` for PR-01",
            target_mode.target,
            target_mode.mode
        );
    }

    let merged_answers = merge_answers(None, None, Some(loaded.answers.clone()), None);
    let provider = ShellWizardProvider;
    let req = ProviderRequest {
        target: target_mode.target.clone(),
        mode: target_mode.mode.clone(),
        frontend: frontend.clone(),
        locale: locale.clone(),
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
                target_mode.target,
                target_mode.mode,
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
        let doc = build_answer_document(
            &target_mode,
            &locale,
            &schema_version,
            &merged_answers,
            &plan,
        );
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
                    {"type":"TextBlock","weight":"Bolder","text":"greentic-dev wizard plan"},
                    {"type":"TextBlock","text": format!("target: {} mode: {}", plan.metadata.target, plan.metadata.mode)},
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
            other => {
                out.push_str(&format!("{}. {:?}\n", idx + 1, other));
            }
        }
    }
    out
}

fn load_answers_input(
    path: Option<&Path>,
    requested_schema_version: Option<&str>,
    migrate: bool,
) -> Result<LoadedAnswers> {
    let Some(path) = path else {
        return Ok(LoadedAnswers {
            answers: serde_json::Value::Object(Default::default()),
            inferred_target_mode: None,
            inferred_locale: None,
            schema_version: requested_schema_version.map(|v| v.to_string()),
        });
    };

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if is_answer_document(&value) {
        let mut doc: AnswerDocument = serde_json::from_value(value)
            .with_context(|| format!("failed to parse AnswerDocument from {}", path.display()))?;

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
            inferred_target_mode: parse_target_mode_from_wizard_id(&doc.wizard_id),
            inferred_locale: Some(doc.locale.clone()),
            schema_version: Some(doc.schema_version.clone()),
        })
    } else {
        if !value.is_object() {
            bail!(
                "answers input must be a JSON object or AnswerDocument envelope: {}",
                path.display()
            );
        }
        Ok(LoadedAnswers {
            answers: value,
            inferred_target_mode: None,
            inferred_locale: None,
            schema_version: requested_schema_version.map(|v| v.to_string()),
        })
    }
}

fn merge_answers(
    cli_overrides: Option<serde_json::Value>,
    parent_prefill: Option<serde_json::Value>,
    answers_file: Option<serde_json::Value>,
    provider_defaults: Option<serde_json::Value>,
) -> WizardAnswers {
    // Highest -> lowest: CLI overrides, parent prefill, answers file, provider defaults.
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

fn resolve_execution_mode(dry_run: bool, execute: bool) -> Result<ExecutionMode> {
    if dry_run && execute {
        bail!("Choose one of --dry-run or --execute.");
    }
    if execute {
        Ok(ExecutionMode::Execute)
    } else {
        Ok(ExecutionMode::DryRun)
    }
}

fn resolve_target_mode(
    cli_target: Option<String>,
    cli_mode: Option<String>,
    inferred: Option<TargetMode>,
) -> Result<TargetMode> {
    match (cli_target, cli_mode, inferred) {
        (Some(target), Some(mode), _) => Ok(TargetMode { target, mode }),
        (None, None, Some(target_mode)) => Ok(target_mode),
        (Some(_), None, _) | (None, Some(_), _) => {
            bail!("target/mode must be provided together or inferred from AnswerDocument wizard_id")
        }
        (None, None, None) => bail!(
            "unable to infer target/mode from answers; pass --target and --mode or provide an AnswerDocument with wizard_id"
        ),
    }
}

fn parse_target_mode_from_wizard_id(wizard_id: &str) -> Option<TargetMode> {
    let rest = wizard_id.strip_prefix(WIZARD_ID_PREFIX)?;
    let (target, mode) = rest.split_once('.')?;
    if target.is_empty() || mode.is_empty() {
        return None;
    }
    Some(TargetMode {
        target: target.to_string(),
        mode: mode.to_string(),
    })
}

fn is_answer_document(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("wizard_id")
        && obj.contains_key("schema_id")
        && obj.contains_key("schema_version")
        && obj.contains_key("locale")
        && obj.contains_key("answers")
}

fn migrate_answer_document(mut doc: AnswerDocument, target_schema_version: &str) -> AnswerDocument {
    doc.schema_version = target_schema_version.to_string();
    doc
}

fn build_answer_document(
    target_mode: &TargetMode,
    locale: &str,
    schema_version: &str,
    answers: &WizardAnswers,
    plan: &WizardPlan,
) -> AnswerDocument {
    AnswerDocument {
        wizard_id: format!(
            "{}{}.{}",
            WIZARD_ID_PREFIX, target_mode.target, target_mode.mode
        ),
        schema_id: format!("greentic-dev.{}.{}", target_mode.target, target_mode.mode),
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

    use super::{
        ExecutionMode, TargetMode, build_answer_document, merge_answers,
        parse_target_mode_from_wizard_id, resolve_execution_mode,
    };
    use crate::wizard::plan::{WizardFrontend, WizardPlan, WizardPlanMetadata};
    use serde_json::json;

    #[test]
    fn mode_defaults_to_dry_run() {
        let mode = resolve_execution_mode(false, false).unwrap();
        assert_eq!(mode, ExecutionMode::DryRun);
    }

    #[test]
    fn mode_rejects_both_flags() {
        let err = resolve_execution_mode(true, true).unwrap_err().to_string();
        assert!(err.contains("Choose one of --dry-run or --execute."));
    }

    #[test]
    fn answer_precedence_parent_over_file() {
        let merged = merge_answers(
            None,
            Some(json!({"foo":"parent","bar":"parent"})),
            Some(json!({"foo":"file","baz":"file"})),
            None,
        );
        assert_eq!(merged.data["foo"], "parent");
        assert_eq!(merged.data["bar"], "parent");
        assert_eq!(merged.data["baz"], "file");
    }

    #[test]
    fn answer_precedence_cli_over_parent() {
        let merged = merge_answers(
            Some(json!({"foo":"cli"})),
            Some(json!({"foo":"parent"})),
            Some(json!({"foo":"file"})),
            None,
        );
        assert_eq!(merged.data["foo"], "cli");
    }

    #[test]
    fn answer_precedence_file_over_defaults() {
        let merged = merge_answers(
            None,
            None,
            Some(json!({"foo":"file"})),
            Some(json!({"foo":"default","bar":"default"})),
        );
        assert_eq!(merged.data["foo"], "file");
        assert_eq!(merged.data["bar"], "default");
    }

    #[test]
    fn parse_target_mode_from_wizard_id_round_trip() {
        let parsed = parse_target_mode_from_wizard_id("greentic-dev.wizard.pack.build").unwrap();
        assert_eq!(parsed.target, "pack");
        assert_eq!(parsed.mode, "build");
    }

    #[test]
    fn build_answer_document_sets_identity_fields() {
        let answers = merge_answers(None, None, Some(json!({"in":"."})), None);
        let plan = WizardPlan {
            plan_version: 1,
            created_at: None,
            metadata: WizardPlanMetadata {
                target: "pack".to_string(),
                mode: "build".to_string(),
                locale: "en-US".to_string(),
                frontend: WizardFrontend::Json,
            },
            inputs: BTreeMap::from([(
                "resolved_versions.greentic-pack".to_string(),
                "greentic-pack 0.1".to_string(),
            )]),
            steps: vec![],
        };

        let doc = build_answer_document(
            &TargetMode {
                target: "pack".to_string(),
                mode: "build".to_string(),
            },
            "en-US",
            "1.0.0",
            &answers,
            &plan,
        );

        assert_eq!(doc.wizard_id, "greentic-dev.wizard.pack.build");
        assert_eq!(doc.schema_id, "greentic-dev.pack.build");
        assert_eq!(doc.schema_version, "1.0.0");
        assert_eq!(doc.locale, "en-US");
        assert_eq!(doc.answers["in"], ".");
        assert_eq!(
            doc.locks.get("resolved_versions.greentic-pack"),
            Some(&"greentic-pack 0.1".to_string())
        );
    }
}
