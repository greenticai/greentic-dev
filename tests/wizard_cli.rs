use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
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
