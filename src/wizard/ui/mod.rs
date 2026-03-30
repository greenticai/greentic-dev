mod assets;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::cli::WizardLaunchArgs;
use crate::i18n;
use crate::passthrough::resolve_binary;

// ---------------------------------------------------------------------------
// Question schema types
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct FormStep {
    id: String,
    title: String,
    description: String,
    fields: Vec<FormField>,
}

#[derive(Serialize, Clone)]
struct FormField {
    id: String,
    label: String,
    kind: FieldKind,
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends_on: Option<FieldDependency>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum FieldKind {
    Text,
    Select,
    Boolean,
}

#[derive(Serialize, Clone)]
struct Choice {
    value: String,
    label: String,
}

#[derive(Serialize, Clone)]
struct FieldDependency {
    field: String,
    value: String,
}

// ---------------------------------------------------------------------------
// Wizard definitions
// ---------------------------------------------------------------------------

fn pack_create_steps() -> Vec<FormStep> {
    vec![
        FormStep {
            id: "pack_basics".into(),
            title: "Create Application Pack".into(),
            description: "Set up a new application pack with flows and components.".into(),
            fields: vec![
                FormField {
                    id: "create_pack_id".into(),
                    label: "Pack ID".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("my-app-pack".into()),
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "pack_dir".into(),
                    label: "Pack Directory".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: Some("./".into()),
                    placeholder: Some("./my-app-pack".into()),
                    choices: vec![],
                    depends_on: None,
                },
            ],
        },
        FormStep {
            id: "pack_options".into(),
            title: "Build Options".into(),
            description: "Select which steps to run after creating the pack.".into(),
            fields: vec![
                FormField {
                    id: "run_doctor".into(),
                    label: "Run doctor (validate pack)".into(),
                    kind: FieldKind::Boolean,
                    required: false,
                    default_value: Some("true".into()),
                    placeholder: None,
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "run_build".into(),
                    label: "Run build (compile pack)".into(),
                    kind: FieldKind::Boolean,
                    required: false,
                    default_value: Some("true".into()),
                    placeholder: None,
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "sign".into(),
                    label: "Sign package".into(),
                    kind: FieldKind::Boolean,
                    required: false,
                    default_value: Some("false".into()),
                    placeholder: None,
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "sign_key_path".into(),
                    label: "Signing key path".into(),
                    kind: FieldKind::Text,
                    required: false,
                    default_value: None,
                    placeholder: Some("./signing.key".into()),
                    choices: vec![],
                    depends_on: Some(FieldDependency {
                        field: "sign".into(),
                        value: "true".into(),
                    }),
                },
            ],
        },
    ]
}

fn bundle_create_steps() -> Vec<FormStep> {
    vec![
        FormStep {
            id: "bundle_basics".into(),
            title: "Create Bundle".into(),
            description: "Set up a new production bundle workspace.".into(),
            fields: vec![
                FormField {
                    id: "bundle_name".into(),
                    label: "Bundle Name".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("My Production Bundle".into()),
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "bundle_id".into(),
                    label: "Bundle ID".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("my-bundle".into()),
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "output_dir".into(),
                    label: "Output Directory".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("~/.greentic/bundles/my-bundle/".into()),
                    choices: vec![],
                    depends_on: None,
                },
            ],
        },
        FormStep {
            id: "bundle_packs".into(),
            title: "Application Packs".into(),
            description: "Add application packs to include in this bundle.".into(),
            fields: vec![
                FormField {
                    id: "app_pack_reference".into(),
                    label: "App Pack Reference".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("oci://ghcr.io/greenticai/my-pack:latest".into()),
                    choices: vec![],
                    depends_on: None,
                },
                FormField {
                    id: "app_pack_scope".into(),
                    label: "Scope".into(),
                    kind: FieldKind::Select,
                    required: true,
                    default_value: Some("global".into()),
                    placeholder: None,
                    choices: vec![
                        Choice {
                            value: "global".into(),
                            label: "Global".into(),
                        },
                        Choice {
                            value: "tenant".into(),
                            label: "Tenant".into(),
                        },
                        Choice {
                            value: "tenant_team".into(),
                            label: "Tenant + Team".into(),
                        },
                    ],
                    depends_on: None,
                },
            ],
        },
        FormStep {
            id: "bundle_options".into(),
            title: "Bundle Options".into(),
            description: "Configure additional bundle settings.".into(),
            fields: vec![FormField {
                id: "enable_bundle_assets".into(),
                label: "Enable bundle assets capability".into(),
                kind: FieldKind::Boolean,
                required: false,
                default_value: Some("false".into()),
                placeholder: None,
                choices: vec![],
                depends_on: None,
            }],
        },
    ]
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

struct UiState {
    locale: String,
    wizard_type: Mutex<Option<String>>,
    answers: Mutex<BTreeMap<String, serde_json::Value>>,
    execution_output: Mutex<Option<ExecutionResult>>,
    shutdown_tx: broadcast::Sender<()>,
}

#[derive(Serialize, Clone)]
struct ExecutionResult {
    success: bool,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LauncherOptions {
    title: String,
    options: Vec<LauncherOption>,
    locale: String,
}

#[derive(Serialize)]
struct LauncherOption {
    value: String,
    label: String,
}

#[derive(Deserialize)]
struct SelectRequest {
    selected_action: String,
}

#[derive(Serialize)]
struct WizardStepsResponse {
    wizard_type: String,
    steps: Vec<FormStep>,
}

#[derive(Deserialize)]
struct SubmitAnswersRequest {
    answers: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn launch_ui(args: &WizardLaunchArgs) -> Result<Option<serde_json::Value>> {
    let locale = i18n::select_locale(args.locale.as_deref());

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { run_server(&locale).await })?;

    // Execution happens inside the server now, no fallback to terminal
    Ok(None)
}

async fn run_server(locale: &str) -> Result<()> {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let state = Arc::new(UiState {
        locale: locale.to_string(),
        wizard_type: Mutex::new(None),
        answers: Mutex::new(BTreeMap::new()),
        execution_output: Mutex::new(None),
        shutdown_tx: shutdown_tx.clone(),
    });

    let router = build_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}");

    eprintln!(
        "{}",
        i18n::tf(locale, "cli.wizard.ui.started", &[("url", url.clone())])
    );
    let _ = open::that(&url);

    let mut shutdown_rx = shutdown_tx.subscribe();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
        })
        .await?;

    Ok(())
}

fn build_router(state: Arc<UiState>) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/app.js", get(serve_js))
        .route("/style.css", get(serve_css))
        .route("/api/launcher/options", get(get_launcher_options))
        .route("/api/launcher/select", post(post_launcher_select))
        .route("/api/wizard/steps", get(get_wizard_steps))
        .route("/api/wizard/submit", post(post_submit_answers))
        .route("/api/wizard/execute", post(post_execute))
        .route("/api/wizard/result", get(get_execution_result))
        .route("/api/shutdown", post(post_shutdown))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Static asset handlers
// ---------------------------------------------------------------------------

async fn serve_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        assets::INDEX_HTML,
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        assets::APP_JS,
    )
}

async fn serve_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        assets::STYLE_CSS,
    )
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

async fn get_launcher_options(State(state): State<Arc<UiState>>) -> Json<LauncherOptions> {
    let locale = &state.locale;
    Json(LauncherOptions {
        title: i18n::t(locale, "cli.wizard.launcher.title"),
        options: vec![
            LauncherOption {
                value: "pack".into(),
                label: i18n::t(locale, "cli.wizard.launcher.option_pack"),
            },
            LauncherOption {
                value: "bundle".into(),
                label: i18n::t(locale, "cli.wizard.launcher.option_bundle"),
            },
        ],
        locale: locale.clone(),
    })
}

async fn post_launcher_select(
    State(state): State<Arc<UiState>>,
    Json(req): Json<SelectRequest>,
) -> Json<StatusResponse> {
    *state.wizard_type.lock().unwrap() = Some(req.selected_action);
    *state.answers.lock().unwrap() = BTreeMap::new();
    *state.execution_output.lock().unwrap() = None;
    Json(StatusResponse {
        status: "ok".into(),
        message: None,
    })
}

async fn get_wizard_steps(State(state): State<Arc<UiState>>) -> Json<Option<WizardStepsResponse>> {
    let wizard_type = state.wizard_type.lock().unwrap().clone();
    let Some(wt) = wizard_type else {
        return Json(None);
    };
    let steps = match wt.as_str() {
        "pack" => pack_create_steps(),
        "bundle" => bundle_create_steps(),
        _ => vec![],
    };
    Json(Some(WizardStepsResponse {
        wizard_type: wt,
        steps,
    }))
}

async fn post_submit_answers(
    State(state): State<Arc<UiState>>,
    Json(req): Json<SubmitAnswersRequest>,
) -> Json<StatusResponse> {
    let mut answers = state.answers.lock().unwrap();
    for (k, v) in req.answers {
        answers.insert(k, v);
    }
    Json(StatusResponse {
        status: "ok".into(),
        message: None,
    })
}

async fn post_execute(State(state): State<Arc<UiState>>) -> Json<StatusResponse> {
    let wizard_type = state.wizard_type.lock().unwrap().clone();
    let answers = state.answers.lock().unwrap().clone();
    let locale = state.locale.clone();

    let Some(wt) = wizard_type else {
        return Json(StatusResponse {
            status: "error".into(),
            message: Some("No wizard type selected".into()),
        });
    };

    let result = tokio::task::spawn_blocking(move || execute_wizard(&wt, &answers, &locale))
        .await
        .unwrap_or_else(|e| ExecutionResult {
            success: false,
            stdout: String::new(),
            stderr: format!("Task panicked: {e}"),
            exit_code: None,
        });

    let success = result.success;
    *state.execution_output.lock().unwrap() = Some(result);

    Json(StatusResponse {
        status: if success { "ok" } else { "error" }.into(),
        message: None,
    })
}

async fn get_execution_result(State(state): State<Arc<UiState>>) -> Json<Option<ExecutionResult>> {
    Json(state.execution_output.lock().unwrap().clone())
}

async fn post_shutdown(State(state): State<Arc<UiState>>) -> Json<StatusResponse> {
    let _ = state.shutdown_tx.send(());
    Json(StatusResponse {
        status: "shutting_down".into(),
        message: None,
    })
}

// ---------------------------------------------------------------------------
// Wizard execution
// ---------------------------------------------------------------------------

fn execute_wizard(
    wizard_type: &str,
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> ExecutionResult {
    let program = match wizard_type {
        "pack" => "greentic-pack",
        "bundle" => "greentic-bundle",
        other => {
            return ExecutionResult {
                success: false,
                stdout: String::new(),
                stderr: format!("Unknown wizard type: {other}"),
                exit_code: None,
            };
        }
    };

    let bin = match resolve_binary(program) {
        Ok(b) => b,
        Err(e) => {
            return ExecutionResult {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to resolve {program}: {e}"),
                exit_code: None,
            };
        }
    };

    let doc = build_answer_document(wizard_type, answers, locale);
    let tmp = match write_temp_answers(&doc) {
        Ok(t) => t,
        Err(e) => {
            return ExecutionResult {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to write temp answers: {e}"),
                exit_code: None,
            };
        }
    };

    let mut args = if program == "greentic-bundle" {
        vec![
            "--locale".to_string(),
            locale.to_string(),
            "wizard".to_string(),
        ]
    } else {
        vec!["wizard".to_string()]
    };
    args.extend_from_slice(&[
        "apply".to_string(),
        "--answers".to_string(),
        tmp.display().to_string(),
    ]);

    match Command::new(&bin)
        .args(&args)
        .env("LANG", locale)
        .env("LC_ALL", locale)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => ExecutionResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
        },
        Err(e) => ExecutionResult {
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to execute {program}: {e}"),
            exit_code: None,
        },
    }
}

fn build_answer_document(
    wizard_type: &str,
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let (wizard_id, schema_id) = match wizard_type {
        "pack" => ("greentic-pack.wizard.run", "greentic-pack.wizard.answers"),
        "bundle" => (
            "greentic-bundle.wizard.run",
            "greentic-bundle.wizard.answers",
        ),
        _ => ("unknown", "unknown"),
    };

    serde_json::json!({
        "wizard_id": wizard_id,
        "schema_id": schema_id,
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": answers,
        "locks": {}
    })
}

fn write_temp_answers(doc: &serde_json::Value) -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("greentic-wizard-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string_pretty(doc)?)?;
    Ok(path)
}
