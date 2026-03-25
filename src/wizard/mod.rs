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
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::cli::{WizardApplyArgs, WizardLaunchArgs, WizardValidateArgs};
use crate::i18n;
use crate::passthrough::resolve_binary;
use crate::wizard::executor::ExecuteOptions;
use crate::wizard::plan::{WizardAnswers, WizardFrontend, WizardPlan};
use crate::wizard::provider::{ProviderRequest, ShellWizardProvider, WizardProvider};

const DEFAULT_LOCALE: &str = "en-US";
const DEFAULT_SCHEMA_VERSION: &str = "1.0.0";
const WIZARD_ID: &str = "greentic-dev.wizard.launcher.main";
const SCHEMA_ID: &str = "greentic-dev.launcher.main";
const BUNDLE_WIZARD_ID_PREFIX: &str = "greentic-bundle.";
const PACK_WIZARD_ID_PREFIX: &str = "greentic-pack.";
const EMBEDDED_WIZARD_ROOT_ZERO_ACTION_ENV: &str = "GREENTIC_WIZARD_ROOT_ZERO_ACTION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    DryRun,
    Execute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LauncherMenuChoice {
    Pack,
    Bundle,
    MainMenu,
    Exit,
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
    locks: serde_json::Map<String, serde_json::Value>,
}

pub fn launch(args: WizardLaunchArgs) -> Result<()> {
    let mode = if args.dry_run {
        ExecutionMode::DryRun
    } else {
        ExecutionMode::Execute
    };

    if let Some(answers_path) = args.answers.as_deref() {
        let loaded =
            load_answer_document(answers_path, args.schema_version.as_deref(), args.migrate)?;

        // When --answers is provided, imply --yes --non-interactive for automation
        return run_from_inputs(
            args.frontend,
            args.locale,
            loaded,
            args.out,
            mode,
            true,
            true,
            args.unsafe_commands,
            args.allow_destructive,
            args.emit_answers,
            args.schema_version,
        );
    }

    let locale = i18n::select_locale(args.locale.as_deref());
    if mode == ExecutionMode::DryRun {
        let Some(answers) = prompt_launcher_answers(mode, &locale)? else {
            return Ok(());
        };
        let loaded = LoadedAnswers {
            answers,
            inferred_locale: None,
            schema_version: args.schema_version.clone(),
        };

        return run_from_inputs(
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
        );
    }

    loop {
        let Some(answers) = prompt_launcher_answers(mode, &locale)? else {
            return Ok(());
        };

        run_interactive_delegate(
            &answers,
            &locale,
            args.emit_answers.as_deref(),
            args.schema_version.as_deref(),
        )?;
        if args.emit_answers.is_some() {
            return Ok(());
        }
    }
}

fn run_interactive_delegate(
    answers: &serde_json::Value,
    locale: &str,
    emit_answers: Option<&Path>,
    requested_schema_version: Option<&str>,
) -> Result<()> {
    let selected_action = answers
        .get("selected_action")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required answers.selected_action"))?;

    let program = match selected_action {
        "pack" => "greentic-pack",
        "bundle" => "greentic-bundle",
        other => bail!("unsupported selected_action `{other}`; expected `pack` or `bundle`"),
    };

    let bin = resolve_binary(program)?;
    let delegated_emit = delegated_emit_capture(emit_answers)?;
    let mut command = Command::new(&bin);
    command
        .args(interactive_delegate_args(
            program,
            locale,
            delegated_emit.path.as_deref(),
        ))
        .env("LANG", locale)
        .env("LC_ALL", locale)
        .env("LC_MESSAGES", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if program == "greentic-bundle" {
        command.env(EMBEDDED_WIZARD_ROOT_ZERO_ACTION_ENV, "back");
    }
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("failed to execute {}: {e}", bin.display()))?;
    if !status.success() {
        bail!(
            "wizard step command failed: {} {:?} (exit code {:?})",
            program,
            ["wizard"],
            status.code()
        );
    }

    if let (Some(output_path), Some(delegated_emit_path)) =
        (emit_answers, delegated_emit.path.as_deref())
    {
        let delegated_doc = read_answer_document(delegated_emit_path)?;
        let Some(delegated_action) = delegated_selected_action(&delegated_doc) else {
            bail!(
                "unsupported delegated wizard_id `{}` in {}; expected `greentic-pack.*` or `greentic-bundle.*`",
                delegated_doc.wizard_id,
                delegated_emit_path.display()
            );
        };
        if delegated_action != selected_action {
            bail!(
                "delegated answers wizard_id `{}` did not match selected_action `{selected_action}`",
                delegated_doc.wizard_id
            );
        }
        let schema_version = requested_schema_version.unwrap_or(DEFAULT_SCHEMA_VERSION);
        let launcher_doc = build_interactive_answer_document(
            locale,
            schema_version,
            selected_action,
            &delegated_doc,
        );
        write_answer_document(output_path, &launcher_doc)?;
    }

    Ok(())
}

fn interactive_delegate_args(
    program: &str,
    locale: &str,
    emit_answers: Option<&Path>,
) -> Vec<String> {
    let mut args = if program == "greentic-bundle" {
        vec![
            "--locale".to_string(),
            locale.to_string(),
            "wizard".to_string(),
        ]
    } else {
        vec!["wizard".to_string()]
    };
    if let Some(path) = emit_answers {
        args.push("run".to_string());
        args.push("--emit-answers".to_string());
        args.push(path.display().to_string());
    }
    args
}

pub fn validate(args: WizardValidateArgs) -> Result<()> {
    let loaded = load_answer_document(&args.answers, args.schema_version.as_deref(), args.migrate)?;

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
    let loaded = load_answer_document(&args.answers, args.schema_version.as_deref(), args.migrate)?;

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
    let delegated_answers_path = persist_delegated_answers_if_present(
        &paths_for_provider(out.as_deref())?,
        &merged_answers,
    )?;
    let provider = ShellWizardProvider;
    let req = ProviderRequest {
        frontend: frontend.clone(),
        locale: locale.clone(),
        dry_run: mode == ExecutionMode::DryRun,
        answers: merged_answers.clone(),
        delegated_answers_path,
    };
    let mut plan = provider.build_plan(&req)?;

    let out_dir = persistence::resolve_out_dir(out.as_deref());
    let paths = persistence::prepare_dir(&out_dir)?;
    persistence::persist_plan_and_answers(&paths, &merged_answers, &plan)?;

    render_plan(&plan)?;

    if mode == ExecutionMode::Execute {
        confirm::ensure_execute_allowed(
            &crate::i18n::tf(
                &locale,
                "runtime.wizard.confirm.summary",
                &[
                    ("target", plan.metadata.target.clone()),
                    ("mode", plan.metadata.mode.clone()),
                    ("step_count", plan.steps.len().to_string()),
                ],
            ),
            yes,
            non_interactive,
            &locale,
        )?;
        let report = executor::execute(
            &plan,
            &paths.exec_log_path,
            &ExecuteOptions {
                unsafe_commands,
                allow_destructive,
                locale: locale.clone(),
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

fn paths_for_provider(out: Option<&Path>) -> Result<persistence::PersistedPaths> {
    let out_dir = persistence::resolve_out_dir(out);
    persistence::prepare_dir(&out_dir)
}

fn persist_delegated_answers_if_present(
    paths: &persistence::PersistedPaths,
    answers: &WizardAnswers,
) -> Result<Option<PathBuf>> {
    let Some(delegated_answers) = answers.data.get("delegate_answer_document") else {
        return Ok(None);
    };
    if !delegated_answers.is_object() {
        bail!("answers.delegate_answer_document must be a JSON object");
    }
    persistence::persist_delegated_answers(&paths.delegated_answers_path, delegated_answers)?;
    Ok(Some(paths.delegated_answers_path.clone()))
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

fn prompt_launcher_answers(mode: ExecutionMode, locale: &str) -> Result<Option<serde_json::Value>> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive {
        bail!(
            "{}",
            i18n::t(locale, "cli.wizard.error.interactive_required")
        );
    }

    loop {
        eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.title"));
        eprintln!();
        eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.option_pack"));
        eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.option_bundle"));
        eprintln!("{}", i18n::t(locale, "cli.wizard.launcher.option_exit"));
        eprintln!();
        eprint!("{}", i18n::t(locale, "cli.wizard.launcher.select_option"));
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        match parse_launcher_menu_choice(input.trim(), true, locale)? {
            LauncherMenuChoice::Pack => return Ok(Some(build_launcher_answers(mode, "pack"))),
            LauncherMenuChoice::Bundle => return Ok(Some(build_launcher_answers(mode, "bundle"))),
            LauncherMenuChoice::MainMenu => {
                eprintln!();
                continue;
            }
            LauncherMenuChoice::Exit => return Ok(None),
        }
    }
}

fn parse_launcher_menu_choice(
    input: &str,
    in_main_menu: bool,
    locale: &str,
) -> Result<LauncherMenuChoice> {
    match input.trim() {
        "1" if in_main_menu => Ok(LauncherMenuChoice::Pack),
        "2" if in_main_menu => Ok(LauncherMenuChoice::Bundle),
        "0" if in_main_menu => Ok(LauncherMenuChoice::Exit),
        "0" => Ok(LauncherMenuChoice::MainMenu),
        "m" | "M" => Ok(LauncherMenuChoice::MainMenu),
        _ => bail!("{}", i18n::t(locale, "cli.wizard.error.invalid_selection")),
    }
}

fn build_launcher_answers(mode: ExecutionMode, selected_action: &str) -> serde_json::Value {
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
    serde_json::Value::Object(answers)
}

fn load_answer_document(
    path_or_url: &str,
    requested_schema_version: Option<&str>,
    migrate: bool,
) -> Result<LoadedAnswers> {
    let mut doc = read_answer_document_from_path_or_url(path_or_url)?;
    if is_launcher_answer_document(&doc) {
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
                path_or_url
            );
        }

        return Ok(LoadedAnswers {
            answers: doc.answers.clone(),
            inferred_locale: Some(doc.locale),
            schema_version: Some(doc.schema_version),
        });
    }

    if let Some(selected_action) = delegated_selected_action(&doc) {
        return Ok(LoadedAnswers {
            answers: wrap_delegated_answer_document(selected_action, &doc),
            inferred_locale: Some(doc.locale),
            schema_version: Some(
                requested_schema_version
                    .unwrap_or(DEFAULT_SCHEMA_VERSION)
                    .to_string(),
            ),
        });
    }

    validate_answer_document_identity(&doc, path_or_url)?;
    unreachable!("launcher identity validation must error for unsupported documents");
}

fn read_answer_document(path: &Path) -> Result<AnswerDocument> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    serde_json::from_value(value)
        .with_context(|| format!("failed to parse AnswerDocument from {}", path.display()))
}

fn read_answer_document_from_path_or_url(path_or_url: &str) -> Result<AnswerDocument> {
    let raw = if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        // Fetch from remote URL
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .with_context(|| "failed to create HTTP client")?;
        let response = client
            .get(path_or_url)
            .send()
            .with_context(|| format!("failed to fetch {}", path_or_url))?;
        if !response.status().is_success() {
            bail!(
                "failed to fetch {}: HTTP {}",
                path_or_url,
                response.status()
            );
        }
        response
            .text()
            .with_context(|| format!("failed to read response from {}", path_or_url))?
    } else {
        let path = Path::new(path_or_url);
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path_or_url))?;
    serde_json::from_value(value)
        .with_context(|| format!("failed to parse AnswerDocument from {}", path_or_url))
}

fn validate_answer_document_identity(doc: &AnswerDocument, path_or_url: &str) -> Result<()> {
    if !is_launcher_answer_document(doc) {
        bail!(
            "unsupported wizard_id `{}` in {}; expected `{}`",
            doc.wizard_id,
            path_or_url,
            WIZARD_ID
        );
    }
    if doc.schema_id != SCHEMA_ID {
        bail!(
            "unsupported schema_id `{}` in {}; expected `{}`",
            doc.schema_id,
            path_or_url,
            SCHEMA_ID
        );
    }
    Ok(())
}

fn is_launcher_answer_document(doc: &AnswerDocument) -> bool {
    doc.wizard_id == WIZARD_ID && doc.schema_id == SCHEMA_ID
}

fn delegated_selected_action(doc: &AnswerDocument) -> Option<&'static str> {
    if doc.wizard_id.starts_with(BUNDLE_WIZARD_ID_PREFIX) {
        Some("bundle")
    } else if doc.wizard_id.starts_with(PACK_WIZARD_ID_PREFIX) {
        Some("pack")
    } else {
        None
    }
}

fn wrap_delegated_answer_document(
    selected_action: &str,
    doc: &AnswerDocument,
) -> serde_json::Value {
    serde_json::json!({
        "selected_action": selected_action,
        "delegate_answer_document": doc,
    })
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
    let locks = plan
        .inputs
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
        .collect();
    AnswerDocument {
        wizard_id: WIZARD_ID.to_string(),
        schema_id: SCHEMA_ID.to_string(),
        schema_version: schema_version.to_string(),
        locale: locale.to_string(),
        answers: answers.data.clone(),
        locks,
    }
}

fn build_interactive_answer_document(
    locale: &str,
    schema_version: &str,
    selected_action: &str,
    delegated_doc: &AnswerDocument,
) -> AnswerDocument {
    AnswerDocument {
        wizard_id: WIZARD_ID.to_string(),
        schema_id: SCHEMA_ID.to_string(),
        schema_version: schema_version.to_string(),
        locale: locale.to_string(),
        answers: wrap_delegated_answer_document(selected_action, delegated_doc),
        locks: serde_json::Map::new(),
    }
}

struct DelegatedEmitCapture {
    _temp_dir: Option<TempDir>,
    path: Option<PathBuf>,
}

fn delegated_emit_capture(emit_answers: Option<&Path>) -> Result<DelegatedEmitCapture> {
    let Some(_) = emit_answers else {
        return Ok(DelegatedEmitCapture {
            _temp_dir: None,
            path: None,
        });
    };
    let temp_dir = tempfile::Builder::new()
        .prefix("greentic-dev-wizard-delegate-")
        .tempdir()
        .context("failed to create tempdir for delegated answers capture")?;
    let path = temp_dir.path().join("delegated-answers.json");
    Ok(DelegatedEmitCapture {
        _temp_dir: Some(temp_dir),
        path: Some(path),
    })
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
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        AnswerDocument, LauncherMenuChoice, SCHEMA_ID, WIZARD_ID, build_answer_document,
        build_interactive_answer_document, build_launcher_answers, interactive_delegate_args,
        is_launcher_answer_document, merge_answers, parse_launcher_menu_choice,
        run_interactive_delegate, validate_answer_document_identity,
        wrap_delegated_answer_document,
    };
    use crate::wizard::plan::{WizardFrontend, WizardPlan, WizardPlanMetadata};

    fn write_stub_bin(dir: &Path, name: &str, body: &str) -> PathBuf {
        #[cfg(windows)]
        let path = dir.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let path = dir.join(name);

        #[cfg(windows)]
        let script = format!("@echo off\r\n{body}\r\n");
        #[cfg(not(windows))]
        let script = format!("#!/bin/sh\n{body}\n");

        fs::write(&path, script).expect("write stub");
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).expect("set perms");
        }
        path
    }

    fn prepend_path(dir: &Path) -> String {
        let old = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ';' } else { ':' };
        format!("{}{}{}", dir.display(), sep, old)
    }

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
            Some(&json!("greentic-pack 0.1"))
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
            locks: serde_json::Map::new(),
        };
        let err = validate_answer_document_identity(&doc, "answers.json").unwrap_err();
        assert!(err.to_string().contains("unsupported wizard_id"));
    }

    #[test]
    fn launcher_identity_matches_expected_pair() {
        let doc = AnswerDocument {
            wizard_id: WIZARD_ID.to_string(),
            schema_id: SCHEMA_ID.to_string(),
            schema_version: "1.0.0".to_string(),
            locale: "en-US".to_string(),
            answers: json!({}),
            locks: serde_json::Map::new(),
        };
        assert!(is_launcher_answer_document(&doc));
    }

    #[test]
    fn wrap_delegated_bundle_document_builds_launcher_shape() {
        let doc = AnswerDocument {
            wizard_id: "greentic-bundle.wizard.main".to_string(),
            schema_id: "greentic-bundle.main".to_string(),
            schema_version: "1.0.0".to_string(),
            locale: "en-US".to_string(),
            answers: json!({"selected_action":"create"}),
            locks: serde_json::Map::new(),
        };
        let wrapped = wrap_delegated_answer_document("bundle", &doc);
        assert_eq!(wrapped["selected_action"], "bundle");
        assert_eq!(
            wrapped["delegate_answer_document"]["wizard_id"],
            "greentic-bundle.wizard.main"
        );
    }

    #[test]
    fn parse_main_menu_navigation_keys() {
        assert_eq!(
            parse_launcher_menu_choice("1", true, "en-US").expect("parse"),
            LauncherMenuChoice::Pack
        );
        assert_eq!(
            parse_launcher_menu_choice("2", true, "en-US").expect("parse"),
            LauncherMenuChoice::Bundle
        );
        assert_eq!(
            parse_launcher_menu_choice("0", true, "en-US").expect("parse"),
            LauncherMenuChoice::Exit
        );
        assert_eq!(
            parse_launcher_menu_choice("M", true, "en-US").expect("parse"),
            LauncherMenuChoice::MainMenu
        );
    }

    #[test]
    fn parse_nested_menu_zero_returns_to_main_menu() {
        assert_eq!(
            parse_launcher_menu_choice("0", false, "en-US").expect("parse"),
            LauncherMenuChoice::MainMenu
        );
    }

    #[test]
    fn build_launcher_answers_includes_selected_action() {
        let answers = build_launcher_answers(super::ExecutionMode::DryRun, "bundle");
        assert_eq!(answers["selected_action"], "bundle");
        assert!(answers.get("delegate_answer_document").is_some());
    }

    #[test]
    fn build_interactive_answer_document_wraps_delegate() {
        let delegated = AnswerDocument {
            wizard_id: "greentic-bundle.wizard.main".to_string(),
            schema_id: "greentic-bundle.main".to_string(),
            schema_version: "1.0.0".to_string(),
            locale: "en-US".to_string(),
            answers: json!({"selected_action":"create"}),
            locks: serde_json::Map::new(),
        };

        let doc = build_interactive_answer_document("en-US", "1.2.3", "bundle", &delegated);

        assert_eq!(doc.wizard_id, WIZARD_ID);
        assert_eq!(doc.schema_id, SCHEMA_ID);
        assert_eq!(doc.schema_version, "1.2.3");
        assert_eq!(doc.answers["selected_action"], "bundle");
        assert_eq!(
            doc.answers["delegate_answer_document"]["wizard_id"],
            "greentic-bundle.wizard.main"
        );
    }

    #[test]
    fn bundle_delegate_receives_locale_flag() {
        assert_eq!(
            interactive_delegate_args("greentic-bundle", "en-GB", None),
            vec!["--locale", "en-GB", "wizard"]
        );
    }

    #[test]
    fn pack_delegate_keeps_plain_wizard_args() {
        assert_eq!(
            interactive_delegate_args("greentic-pack", "en-GB", None),
            vec!["wizard"]
        );
    }

    #[test]
    fn bundle_delegate_emit_answers_uses_run_subcommand() {
        assert_eq!(
            interactive_delegate_args(
                "greentic-bundle",
                "en-GB",
                Some(Path::new("/tmp/emitted.json"))
            ),
            vec![
                "--locale",
                "en-GB",
                "wizard",
                "run",
                "--emit-answers",
                "/tmp/emitted.json",
            ]
        );
    }

    #[test]
    fn pack_delegate_emit_answers_uses_run_subcommand() {
        assert_eq!(
            interactive_delegate_args(
                "greentic-pack",
                "en-GB",
                Some(Path::new("/tmp/emitted.json"))
            ),
            vec!["wizard", "run", "--emit-answers", "/tmp/emitted.json"]
        );
    }

    #[test]
    fn interactive_bundle_delegate_emit_answers_writes_launcher_document() {
        let tmp = TempDir::new().expect("temp dir");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let emitted = tmp.path().join("answers-envelope.json");
        let runlog = tmp.path().join("bundle-run.log");
        let original_path = std::env::var_os("PATH");

        write_stub_bin(
            &bin_dir,
            "greentic-bundle",
            &format!(
                r#"
echo "$@" > "{}"
if [ "$1" != "--locale" ] || [ "$2" != "en-US" ] || [ "$3" != "wizard" ] || [ "$4" != "run" ] || [ "$5" != "--emit-answers" ]; then
  echo "unexpected argv: $@" >&2
  exit 9
fi
cat > "$6" <<'EOF'
{{
  "wizard_id": "greentic-bundle.wizard.main",
  "schema_id": "greentic-bundle.main",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {{
    "selected_action": "create"
  }},
  "locks": {{}}
}}
EOF
exit 0
"#,
                runlog.display()
            ),
        );

        unsafe {
            std::env::set_var("PATH", prepend_path(&bin_dir));
        }
        let result = run_interactive_delegate(
            &json!({"selected_action":"bundle"}),
            "en-US",
            Some(&emitted),
            Some("1.2.3"),
        );
        if let Some(path) = original_path {
            unsafe {
                std::env::set_var("PATH", path);
            }
        } else {
            unsafe {
                std::env::remove_var("PATH");
            }
        }

        result.expect("interactive delegate succeeds");

        let argv = fs::read_to_string(&runlog).expect("read run log");
        assert!(argv.contains("wizard run --emit-answers"));
        assert!(
            !argv.contains("wizard --emit-answers"),
            "bundle delegate should not receive unsupported bare wizard emit flags"
        );

        let emitted_doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&emitted).expect("read emitted answers"))
                .expect("parse emitted answers");
        assert_eq!(emitted_doc["wizard_id"], WIZARD_ID);
        assert_eq!(emitted_doc["schema_id"], SCHEMA_ID);
        assert_eq!(emitted_doc["schema_version"], "1.2.3");
        assert_eq!(emitted_doc["answers"]["selected_action"], "bundle");
        assert_eq!(
            emitted_doc["answers"]["delegate_answer_document"]["wizard_id"],
            "greentic-bundle.wizard.main"
        );
    }
}
