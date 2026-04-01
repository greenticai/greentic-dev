use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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

fn write_launcher_answers(path: &Path, action: &str) {
    fs::write(
        path,
        format!(
            r#"{{
  "wizard_id": "greentic-dev.wizard.launcher.main",
  "schema_id": "greentic-dev.launcher.main",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {{
    "selected_action": "{action}"
  }},
  "locks": {{}}
}}
"#
        ),
    )
    .expect("write answers doc");
}

fn write_launcher_answers_with_delegate(path: &Path, action: &str, delegate_doc: &str) {
    fs::write(
        path,
        format!(
            r#"{{
  "wizard_id": "greentic-dev.wizard.launcher.main",
  "schema_id": "greentic-dev.launcher.main",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {{
    "selected_action": "{action}",
    "delegate_answer_document": {delegate_doc}
  }},
  "locks": {{}}
}}
"#
        ),
    )
    .expect("write delegated answers doc");
}

fn write_bundle_answers(path: &Path) {
    fs::write(
        path,
        r#"{
  "wizard_id": "greentic-bundle.wizard.main",
  "schema_id": "greentic-bundle.main",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {
    "selected_action": "create"
  },
  "locks": {}
}
"#,
    )
    .expect("write bundle answers doc");
}

#[test]
fn wizard_no_subcommand_requires_interactive_terminal() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.arg("wizard");
    cmd.assert()
        .failure()
        .stderr(contains("wizard launcher requires interactive input"));
}

#[test]
fn wizard_replay_command_removed() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["wizard", "replay", "--help"]);
    cmd.assert()
        .failure()
        .stderr(contains("unrecognized subcommand"));
}

#[test]
fn wizard_validate_answers_document_runs_dry_run_plan() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-validate");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers(&answers_doc, "pack");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "validate",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"target\": \"launcher\""))
        .stdout(contains("\"mode\": \"main\""))
        .stdout(contains("\"program\": \"greentic-pack\""));

    assert!(
        !out.join("exec.log").exists(),
        "dry-run validate should not create exec.log"
    );
}

#[test]
fn wizard_top_level_answers_document_runs_dry_run_plan() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-top-level-validate");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers(&answers_doc, "pack");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--dry-run",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"target\": \"launcher\""))
        .stdout(contains("\"mode\": \"main\""))
        .stdout(contains("\"program\": \"greentic-pack\""));

    assert!(
        !out.join("exec.log").exists(),
        "top-level dry-run should not create exec.log"
    );
}

#[test]
fn wizard_apply_answers_document_executes_delegation() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-apply");
    let runlog = tmp.path().join("apply-runs.log");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers(&answers_doc, "pack");

    write_stub_bin(
        &bin_dir,
        "greentic-pack",
        &format!(
            r#"
if [ "$1" = "--version" ]; then
  echo "greentic-pack 0.4.test"
  exit 0
fi
echo "$@" >> "{}"
exit 0
"#,
            runlog.display()
        ),
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "apply",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let exec_log = fs::read_to_string(out.join("exec.log")).expect("read exec log");
    assert!(exec_log.contains("RUN greentic-pack wizard"));
    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(stub_log.contains("wizard"));
}

#[test]
fn wizard_top_level_answers_document_executes_delegation() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-top-level-apply");
    let runlog = tmp.path().join("top-level-apply-runs.log");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers(&answers_doc, "pack");

    write_stub_bin(
        &bin_dir,
        "greentic-pack",
        &format!(
            r#"
if [ "$1" = "--version" ]; then
  echo "greentic-pack 0.4.test"
  exit 0
fi
echo "$@" >> "{}"
exit 0
"#,
            runlog.display()
        ),
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let exec_log = fs::read_to_string(out.join("exec.log")).expect("read exec log");
    assert!(exec_log.contains("RUN greentic-pack wizard"));
    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(stub_log.contains("wizard"));
}

#[test]
fn wizard_top_level_dry_run_plan_includes_delegated_answers_apply() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-top-level-dry-delegated");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers_with_delegate(
        &answers_doc,
        "bundle",
        r#"{
      "wizard_id": "greentic-bundle.wizard.main",
      "schema_id": "greentic-bundle.main",
      "schema_version": "1.0.0",
      "locale": "en-US",
      "answers": {
        "selected_action": "create"
      },
      "locks": {}
    }"#,
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--dry-run",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"program\": \"greentic-bundle\""))
        .stdout(contains("\"apply\""))
        .stdout(contains("delegated-answers.json"))
        .stdout(contains("\"--dry-run\""));

    let delegated = fs::read_to_string(out.join("delegated-answers.json")).expect("read delegated");
    assert!(delegated.contains("\"wizard_id\": \"greentic-bundle.wizard.main\""));
    assert!(delegated.contains("\"selected_action\": \"create\""));
    assert!(
        !out.join("exec.log").exists(),
        "top-level delegated dry-run should not create exec.log"
    );
}

#[test]
fn wizard_top_level_answers_document_replays_delegated_answers_non_interactively() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-top-level-delegated-apply");
    let runlog = tmp.path().join("top-level-delegated-apply-runs.log");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers_with_delegate(
        &answers_doc,
        "bundle",
        r#"{
      "wizard_id": "greentic-bundle.wizard.main",
      "schema_id": "greentic-bundle.main",
      "schema_version": "1.0.0",
      "locale": "en-US",
      "answers": {
        "selected_action": "create"
      },
      "locks": {}
    }"#,
    );

    write_stub_bin(
        &bin_dir,
        "greentic-bundle",
        &format!(
            r#"
if [ "$1" = "--version" ]; then
  echo "greentic-bundle 0.4.test"
  exit 0
fi
echo "$@" >> "{}"
if [ "$1" != "wizard" ] || [ "$2" != "apply" ] || [ "$3" != "--answers" ]; then
  echo "unexpected argv: $@" >&2
  exit 9
fi
if ! grep -q '"selected_action": "create"' "$4"; then
  echo "delegated answers missing expected payload" >&2
  exit 10
fi
exit 0
"#,
            runlog.display()
        ),
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--yes",
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let exec_log = fs::read_to_string(out.join("exec.log")).expect("read exec log");
    assert!(exec_log.contains("RUN greentic-bundle wizard apply --answers"));
    assert!(exec_log.contains("delegated-answers.json"));
    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(stub_log.contains("wizard apply --answers"));
}

#[test]
fn wizard_top_level_bundle_answer_document_runs_bundle_delegate_dry_run() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-top-level-bundle-direct-dry");
    let answers_doc = tmp.path().join("bundle-answers-doc.json");
    write_bundle_answers(&answers_doc);

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--dry-run",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"program\": \"greentic-bundle\""))
        .stdout(contains("\"apply\""))
        .stdout(contains("delegated-answers.json"));

    let delegated = fs::read_to_string(out.join("delegated-answers.json")).expect("read delegated");
    assert!(delegated.contains("\"wizard_id\": \"greentic-bundle.wizard.main\""));
    assert!(delegated.contains("\"selected_action\": \"create\""));
}

#[test]
fn wizard_top_level_bundle_answer_document_executes_bundle_delegate_non_interactively() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-top-level-bundle-direct-apply");
    let runlog = tmp.path().join("top-level-bundle-direct-apply-runs.log");
    let answers_doc = tmp.path().join("bundle-answers-doc.json");
    write_bundle_answers(&answers_doc);

    write_stub_bin(
        &bin_dir,
        "greentic-bundle",
        &format!(
            r#"
if [ "$1" = "--version" ]; then
  echo "greentic-bundle 0.4.test"
  exit 0
fi
echo "$@" >> "{}"
if [ "$1" != "wizard" ] || [ "$2" != "apply" ] || [ "$3" != "--answers" ]; then
  echo "unexpected argv: $@" >&2
  exit 9
fi
if ! grep -q '"selected_action": "create"' "$4"; then
  echo "delegated answers missing expected payload" >&2
  exit 10
fi
exit 0
"#,
            runlog.display()
        ),
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--yes",
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let exec_log = fs::read_to_string(out.join("exec.log")).expect("read exec log");
    assert!(exec_log.contains("RUN greentic-bundle wizard apply --answers"));
    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(stub_log.contains("wizard apply --answers"));
}

#[test]
fn wizard_apply_rejects_non_launcher_answer_document_ids() {
    let tmp = TempDir::new().expect("temp dir");
    let answers_doc = tmp.path().join("answers-doc.json");
    fs::write(
        &answers_doc,
        r#"{
  "wizard_id": "greentic-dev.wizard.pack.build",
  "schema_id": "greentic-dev.pack.build",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {
    "selected_action": "pack"
  },
  "locks": {}
}
"#,
    )
    .expect("write answers doc");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "apply",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--non-interactive",
    ]);
    cmd.assert()
        .failure()
        .stderr(contains("unsupported wizard_id"));
}

#[test]
fn wizard_apply_emit_answers_writes_launcher_document() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-emit");
    let emitted = tmp.path().join("answers-envelope.json");
    let answers_doc = tmp.path().join("answers-doc.json");
    write_launcher_answers(&answers_doc, "bundle");

    write_stub_bin(
        &bin_dir,
        "greentic-bundle",
        r#"
if [ "$1" = "--version" ]; then
  echo "greentic-bundle 0.4.test"
  exit 0
fi
exit 0
"#,
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "apply",
        "--answers",
        answers_doc.to_str().expect("utf8 path"),
        "--emit-answers",
        emitted.to_str().expect("utf8 path"),
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let envelope = fs::read_to_string(&emitted).expect("read emitted envelope");
    assert!(envelope.contains("\"wizard_id\": \"greentic-dev.wizard.launcher.main\""));
    assert!(envelope.contains("\"schema_id\": \"greentic-dev.launcher.main\""));
    assert!(envelope.contains("\"selected_action\": \"bundle\""));
}

#[test]
fn wizard_schema_combines_pack_and_bundle_delegate_schemas() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");

    write_stub_bin(
        &bin_dir,
        "greentic-pack",
        r#"
if [ "$1" = "wizard" ] && [ "$2" = "--schema" ]; then
  cat <<'JSON'
{"title":"greentic-pack wizard answers","type":"object","properties":{"schema_id":{"const":"greentic-pack.wizard.answers"}}}
JSON
  exit 0
fi
echo "unexpected argv: $@" >&2
exit 9
"#,
    );

    write_stub_bin(
        &bin_dir,
        "greentic-bundle",
        r#"
if [ "$1" = "--locale" ] && [ "$3" = "wizard" ] && [ "$4" = "--schema" ]; then
  cat <<'JSON'
{"title":"greentic-bundle wizard answers","type":"object","properties":{"schema_id":{"const":"greentic-bundle.main"}}}
JSON
  exit 0
fi
echo "unexpected argv: $@" >&2
exit 9
"#,
    );

    let output = cargo_bin_cmd!("greentic-dev")
        .env("PATH", prepend_path(&bin_dir))
        .args(["wizard", "--schema", "--schema-version", "1.2.3"])
        .output()
        .expect("run wizard --schema");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let schema: Value = serde_json::from_slice(&output.stdout).expect("parse schema");
    assert_eq!(
        schema.get("title").and_then(Value::as_str),
        Some("greentic-dev launcher wizard answers")
    );
    assert_eq!(
        schema
            .pointer("/properties/schema_version/const")
            .and_then(Value::as_str),
        Some("1.2.3")
    );
    assert_eq!(
        schema
            .pointer("/properties/answers/properties/selected_action/enum")
            .and_then(Value::as_array)
            .expect("selected_action enum")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        vec!["pack", "bundle"]
    );
    assert_eq!(
        schema
            .pointer("/$defs/greentic_pack_wizard_answers/title")
            .and_then(Value::as_str),
        Some("greentic-pack wizard answers")
    );
    assert_eq!(
        schema
            .pointer("/$defs/greentic_bundle_wizard_answers/title")
            .and_then(Value::as_str),
        Some("greentic-bundle wizard answers")
    );
}
