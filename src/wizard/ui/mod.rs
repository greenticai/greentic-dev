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

fn pack_update_steps() -> Vec<FormStep> {
    vec![FormStep {
        id: "pack_update_basics".into(),
        title: "Update Application Pack".into(),
        description: "Update an existing application pack.".into(),
        fields: vec![
            FormField {
                id: "pack_dir".into(),
                label: "Pack Directory".into(),
                kind: FieldKind::Text,
                required: true,
                default_value: Some(".".into()),
                placeholder: Some("./my-app-pack".into()),
                choices: vec![],
                depends_on: None,
            },
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
    }]
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

fn bundle_update_steps() -> Vec<FormStep> {
    vec![FormStep {
        id: "bundle_update_basics".into(),
        title: "Update Bundle".into(),
        description: "Open an existing bundle directory or .gtbundle artifact to edit.".into(),
        fields: vec![FormField {
            id: "bundle_target".into(),
            label: "Bundle Directory or .gtbundle Path".into(),
            kind: FieldKind::Text,
            required: true,
            default_value: Some(".".into()),
            placeholder: Some("./my-bundle or ./my-bundle.gtbundle".into()),
            choices: vec![],
            depends_on: None,
        }],
    }]
}

fn bundle_path_steps(title: &str, desc: &str) -> Vec<FormStep> {
    vec![FormStep {
        id: "bundle_target".into(),
        title: title.into(),
        description: desc.into(),
        fields: vec![FormField {
            id: "bundle_target".into(),
            label: "Bundle Directory or .gtbundle Path".into(),
            kind: FieldKind::Text,
            required: true,
            default_value: Some(".".into()),
            placeholder: Some("./my-bundle".into()),
            choices: vec![],
            depends_on: None,
        }],
    }]
}

fn ext_catalog_field() -> FormField {
    FormField {
        id: "extension_catalog_ref".into(),
        label: "Extension Catalog Reference".into(),
        kind: FieldKind::Text,
        required: true,
        default_value: Some("file://docs/extensions_capability_packs.catalog.v1.json".into()),
        placeholder: Some("file://, https://, or oci:// catalog reference".into()),
        choices: vec![],
        depends_on: None,
    }
}

fn ext_type_field() -> FormField {
    FormField {
        id: "extension_type_id".into(),
        label: "Extension Type ID".into(),
        kind: FieldKind::Text,
        required: true,
        default_value: None,
        placeholder: Some(
            "e.g. control, deployer, messaging, oauth, secrets, state, events, telemetry".into(),
        ),
        choices: vec![],
        depends_on: None,
    }
}

fn pack_create_ext_steps() -> Vec<FormStep> {
    vec![
        FormStep {
            id: "ext_basics".into(),
            title: "Create Extension Pack".into(),
            description: "Create a new extension pack from a catalog template.".into(),
            fields: vec![
                ext_catalog_field(),
                ext_type_field(),
                FormField {
                    id: "extension_template_id".into(),
                    label: "Extension Template ID".into(),
                    kind: FieldKind::Text,
                    required: true,
                    default_value: None,
                    placeholder: Some("e.g. control-basic, secrets-env".into()),
                    choices: vec![],
                    depends_on: None,
                },
            ],
        },
        FormStep {
            id: "ext_pack_dir".into(),
            title: "Pack Location".into(),
            description: "Where to create the extension pack.".into(),
            fields: vec![FormField {
                id: "pack_dir".into(),
                label: "Pack Directory".into(),
                kind: FieldKind::Text,
                required: true,
                default_value: None,
                placeholder: Some("./my-extension-pack".into()),
                choices: vec![],
                depends_on: None,
            }],
        },
    ]
}

fn pack_update_ext_steps() -> Vec<FormStep> {
    vec![FormStep {
        id: "ext_update".into(),
        title: "Update Extension Pack".into(),
        description: "Update an existing extension pack.".into(),
        fields: vec![
            FormField {
                id: "pack_dir".into(),
                label: "Pack Directory".into(),
                kind: FieldKind::Text,
                required: true,
                default_value: Some(".".into()),
                placeholder: Some("./my-extension-pack".into()),
                choices: vec![],
                depends_on: None,
            },
            ext_catalog_field(),
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
        ],
    }]
}

fn pack_add_ext_steps() -> Vec<FormStep> {
    vec![FormStep {
        id: "ext_add".into(),
        title: "Add Extension to Existing Pack".into(),
        description: "Add an extension entry to an existing pack.".into(),
        fields: vec![
            FormField {
                id: "pack_dir".into(),
                label: "Pack Directory".into(),
                kind: FieldKind::Text,
                required: true,
                default_value: Some(".".into()),
                placeholder: Some("./my-pack".into()),
                choices: vec![],
                depends_on: None,
            },
            ext_catalog_field(),
            ext_type_field(),
        ],
    }]
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

struct UiState {
    locale: String,
    wizard_type: Mutex<Option<String>>,
    sub_action: Mutex<Option<String>>,
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
        sub_action: Mutex::new(None),
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
        .route("/api/wizard/submenu", get(get_submenu_options))
        .route("/api/wizard/submenu/select", post(post_submenu_select))
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
    *state.sub_action.lock().unwrap() = None;
    *state.answers.lock().unwrap() = BTreeMap::new();
    *state.execution_output.lock().unwrap() = None;
    Json(StatusResponse {
        status: "ok".into(),
        message: None,
    })
}

#[derive(Serialize)]
struct SubmenuResponse {
    wizard_type: String,
    title: String,
    options: Vec<SubmenuOption>,
}

#[derive(Serialize)]
struct SubmenuOption {
    value: String,
    label: String,
    description: String,
}

async fn get_submenu_options(State(state): State<Arc<UiState>>) -> Json<Option<SubmenuResponse>> {
    let wizard_type = state.wizard_type.lock().unwrap().clone();
    let Some(wt) = wizard_type else {
        return Json(None);
    };
    let resp = match wt.as_str() {
        "pack" => SubmenuResponse {
            wizard_type: "pack".into(),
            title: "Pack Wizard".into(),
            options: vec![
                SubmenuOption {
                    value: "create_app".into(),
                    label: "Create application pack".into(),
                    description: "Scaffold a new application pack with flows and components."
                        .into(),
                },
                SubmenuOption {
                    value: "update_app".into(),
                    label: "Update application pack".into(),
                    description: "Update an existing application pack.".into(),
                },
                SubmenuOption {
                    value: "create_ext".into(),
                    label: "Create extension pack".into(),
                    description: "Create a new extension pack from a catalog template.".into(),
                },
                SubmenuOption {
                    value: "update_ext".into(),
                    label: "Update extension pack".into(),
                    description: "Update an existing extension pack.".into(),
                },
                SubmenuOption {
                    value: "add_ext".into(),
                    label: "Add extension to existing pack".into(),
                    description: "Add an extension entry to an existing pack.".into(),
                },
            ],
        },
        "bundle" => SubmenuResponse {
            wizard_type: "bundle".into(),
            title: "Bundle Wizard".into(),
            options: vec![
                SubmenuOption {
                    value: "create".into(),
                    label: "Create bundle".into(),
                    description: "Start a new bundle workspace.".into(),
                },
                SubmenuOption {
                    value: "update".into(),
                    label: "Update bundle".into(),
                    description: "Open and edit an existing bundle.".into(),
                },
                SubmenuOption {
                    value: "validate".into(),
                    label: "Validate bundle".into(),
                    description: "Preview the normalized bundle plan without writing files.".into(),
                },
                SubmenuOption {
                    value: "doctor".into(),
                    label: "Doctor".into(),
                    description: "Run doctor checks against a bundle.".into(),
                },
                SubmenuOption {
                    value: "inspect".into(),
                    label: "Inspect".into(),
                    description: "Inspect a bundle directory or .gtbundle artifact.".into(),
                },
            ],
        },
        _ => return Json(None),
    };
    Json(Some(resp))
}

async fn post_submenu_select(
    State(state): State<Arc<UiState>>,
    Json(req): Json<SelectRequest>,
) -> Json<StatusResponse> {
    *state.sub_action.lock().unwrap() = Some(req.selected_action);
    *state.answers.lock().unwrap() = BTreeMap::new();
    Json(StatusResponse {
        status: "ok".into(),
        message: None,
    })
}

async fn get_wizard_steps(State(state): State<Arc<UiState>>) -> Json<Option<WizardStepsResponse>> {
    let wizard_type = state.wizard_type.lock().unwrap().clone();
    let sub_action = state.sub_action.lock().unwrap().clone();
    let Some(wt) = wizard_type else {
        return Json(None);
    };
    let sa = sub_action.unwrap_or_default();
    let steps = match (wt.as_str(), sa.as_str()) {
        ("pack", "create_app") => pack_create_steps(),
        ("pack", "update_app") => pack_update_steps(),
        ("pack", "create_ext") => pack_create_ext_steps(),
        ("pack", "update_ext") => pack_update_ext_steps(),
        ("pack", "add_ext") => pack_add_ext_steps(),
        ("bundle", "create") => bundle_create_steps(),
        ("bundle", "update") => bundle_update_steps(),
        ("bundle", "validate") => bundle_path_steps(
            "Validate Bundle",
            "Preview the normalized bundle plan without writing files.",
        ),
        ("bundle", "doctor") => bundle_path_steps("Doctor", "Run doctor checks against a bundle."),
        ("bundle", "inspect") => {
            bundle_path_steps("Inspect", "Inspect bundle workspace or artifact metadata.")
        }
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
    let sub_action = state.sub_action.lock().unwrap().clone();
    let answers = state.answers.lock().unwrap().clone();
    let locale = state.locale.clone();

    let Some(wt) = wizard_type else {
        return Json(StatusResponse {
            status: "error".into(),
            message: Some("No wizard type selected".into()),
        });
    };

    let sa = sub_action.unwrap_or_default();
    let result = tokio::task::spawn_blocking(move || execute_wizard(&wt, &sa, &answers, &locale))
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
    sub_action: &str,
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> ExecutionResult {
    // Bundle direct commands (doctor/inspect/validate) run without AnswerDocument
    if wizard_type == "bundle" && matches!(sub_action, "doctor" | "inspect" | "validate") {
        return execute_bundle_direct(sub_action, answers, locale);
    }

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

    let doc = build_answer_document(wizard_type, sub_action, answers, locale);
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

    run_command(&bin, &args, locale)
}

fn execute_bundle_direct(
    sub_action: &str,
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> ExecutionResult {
    let bin = match resolve_binary("greentic-bundle") {
        Ok(b) => b,
        Err(e) => {
            return ExecutionResult {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to resolve greentic-bundle: {e}"),
                exit_code: None,
            };
        }
    };

    let target = answers
        .get("bundle_target")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let mut args = vec![
        "--locale".to_string(),
        locale.to_string(),
        sub_action.to_string(),
        "--root".to_string(),
        target.to_string(),
        "--json".to_string(),
    ];

    // validate is done via wizard with dry-run
    if sub_action == "validate" {
        args = vec![
            "--locale".to_string(),
            locale.to_string(),
            "build".to_string(),
            "--root".to_string(),
            target.to_string(),
            "--dry-run".to_string(),
        ];
    }

    run_command(&bin, &args, locale)
}

fn run_command(bin: &std::path::Path, args: &[String], locale: &str) -> ExecutionResult {
    match Command::new(bin)
        .args(args)
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
            stderr: format!("Failed to execute {}: {e}", bin.display()),
            exit_code: None,
        },
    }
}

fn build_answer_document(
    wizard_type: &str,
    sub_action: &str,
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    match (wizard_type, sub_action) {
        ("pack", "create_app") => build_pack_create_answer_document(answers, locale),
        ("pack", "update_app") => build_pack_update_answer_document(answers, locale),
        ("pack", "create_ext") => build_pack_create_ext_answer_document(answers, locale),
        ("pack", "update_ext") => build_pack_update_answer_document(answers, locale),
        ("pack", "add_ext") => build_pack_add_ext_answer_document(answers, locale),
        ("bundle", "create") => build_bundle_answer_document(answers, locale),
        _ => serde_json::json!({}),
    }
}

fn build_pack_create_answer_document(
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let pack_id = answers
        .get("create_pack_id")
        .and_then(|v| v.as_str())
        .unwrap_or("my-pack");
    let pack_dir = answers
        .get("pack_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./");
    let run_doctor = answers
        .get("run_doctor")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let run_build = answers
        .get("run_build")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let sign = answers
        .get("sign")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sign_key_path = answers
        .get("sign_key_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut doc_answers = serde_json::json!({
        "selected_actions": [
            "main.create_application_pack",
            "create_application_pack.start"
        ],
        "create_pack_id": pack_id,
        "create_pack_scaffold": true,
        "pack_dir": pack_dir,
        "run_delegate_flow": false,
        "run_delegate_component": false,
        "run_doctor": run_doctor,
        "run_build": run_build,
        "sign": sign,
        "mode": "interactive"
    });

    if sign && !sign_key_path.is_empty() {
        doc_answers["sign_key_path"] = serde_json::Value::String(sign_key_path.to_string());
    }

    serde_json::json!({
        "wizard_id": "greentic-pack.wizard.run",
        "schema_id": "greentic-pack.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": doc_answers,
        "locks": {}
    })
}

fn build_pack_update_answer_document(
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let pack_dir = answers
        .get("pack_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let run_doctor = answers
        .get("run_doctor")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let run_build = answers
        .get("run_build")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let sign = answers
        .get("sign")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sign_key_path = answers
        .get("sign_key_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut doc_answers = serde_json::json!({
        "selected_actions": [
            "main.update_application_pack",
            "update_application_pack.start"
        ],
        "pack_dir": pack_dir,
        "run_delegate_flow": false,
        "run_delegate_component": false,
        "run_doctor": run_doctor,
        "run_build": run_build,
        "sign": sign,
        "mode": "interactive"
    });

    if sign && !sign_key_path.is_empty() {
        doc_answers["sign_key_path"] = serde_json::Value::String(sign_key_path.to_string());
    }

    serde_json::json!({
        "wizard_id": "greentic-pack.wizard.run",
        "schema_id": "greentic-pack.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": doc_answers,
        "locks": {}
    })
}

fn build_pack_create_ext_answer_document(
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let pack_dir = answers
        .get("pack_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./");
    let catalog_ref = answers
        .get("extension_catalog_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let type_id = answers
        .get("extension_type_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let template_id = answers
        .get("extension_template_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    serde_json::json!({
        "wizard_id": "greentic-pack.wizard.run",
        "schema_id": "greentic-pack.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": {
            "selected_actions": [
                "main.create_extension_pack",
                "create_extension_pack.start"
            ],
            "pack_dir": pack_dir,
            "extension_operation": "create_extension_pack",
            "extension_catalog_ref": catalog_ref,
            "extension_type_id": type_id,
            "extension_template_id": template_id,
            "extension_template_qa_answers": {},
            "extension_edit_answers": {},
            "run_doctor": true,
            "run_build": true,
            "sign": false,
            "mode": "interactive"
        },
        "locks": {}
    })
}

fn build_pack_add_ext_answer_document(
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let pack_dir = answers
        .get("pack_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let catalog_ref = answers
        .get("extension_catalog_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let type_id = answers
        .get("extension_type_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    serde_json::json!({
        "wizard_id": "greentic-pack.wizard.run",
        "schema_id": "greentic-pack.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": {
            "selected_actions": [
                "main.add_extension",
                "add_extension.start"
            ],
            "pack_dir": pack_dir,
            "extension_operation": "add_extension",
            "extension_catalog_ref": catalog_ref,
            "extension_type_id": type_id,
            "extension_edit_answers": {},
            "run_doctor": true,
            "run_build": true,
            "sign": false,
            "mode": "interactive"
        },
        "locks": {}
    })
}

fn build_bundle_answer_document(
    answers: &BTreeMap<String, serde_json::Value>,
    locale: &str,
) -> serde_json::Value {
    let bundle_name = answers
        .get("bundle_name")
        .and_then(|v| v.as_str())
        .unwrap_or("My Bundle");
    let bundle_id = answers
        .get("bundle_id")
        .and_then(|v| v.as_str())
        .unwrap_or("my-bundle");
    let output_dir = answers
        .get("output_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let enable_assets = answers
        .get("enable_bundle_assets")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let capabilities = if enable_assets {
        serde_json::json!(["greentic.capability.bundle_assets_read_v1"])
    } else {
        serde_json::json!([])
    };

    let mut app_packs = Vec::new();
    if let Some(reference) = answers.get("app_pack_reference").and_then(|v| v.as_str())
        && !reference.is_empty()
    {
        let scope = answers
            .get("app_pack_scope")
            .and_then(|v| v.as_str())
            .unwrap_or("global");
        // Derive pack_id from reference (last segment or full ref)
        let pack_id = reference
            .rsplit('/')
            .next()
            .unwrap_or(reference)
            .split(':')
            .next()
            .unwrap_or(reference);
        app_packs.push(serde_json::json!({
            "reference": reference,
            "detected_kind": "reference",
            "pack_id": pack_id,
            "display_name": pack_id,
            "mapping": {
                "scope": scope
            }
        }));
    }

    serde_json::json!({
        "wizard_id": "greentic-bundle.wizard.run",
        "schema_id": "greentic-bundle.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": {
            "mode": "create",
            "bundle_name": bundle_name,
            "bundle_id": bundle_id,
            "output_dir": output_dir,
            "app_pack_entries": app_packs,
            "extension_provider_entries": [],
            "access_rules": [],
            "capabilities": capabilities,
            "advanced_setup": false,
            "setup_execution_intent": false,
            "export_intent": false
        },
        "locks": {}
    })
}

fn write_temp_answers(doc: &serde_json::Value) -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("greentic-wizard-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string_pretty(doc)?)?;
    Ok(path)
}
