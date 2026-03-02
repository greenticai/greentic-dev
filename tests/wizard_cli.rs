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

#[test]
fn wizard_rejects_both_execute_flags() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--dry-run",
        "--execute",
    ]);
    cmd.assert()
        .failure()
        .stderr(contains("Choose one of --dry-run or --execute."));
}

#[test]
fn wizard_dry_run_outputs_stable_plan_fields() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-out");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--frontend",
        "json",
        "--locale",
        "en-US",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"plan_version\": 1"))
        .stdout(contains("\"target\": \"pack\""))
        .stdout(contains("\"mode\": \"build\""));
}

#[test]
fn wizard_replay_roundtrip_dry_run() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-roundtrip");
    let answers = out.join("answers.json");

    let mut run_cmd = cargo_bin_cmd!("greentic-dev");
    run_cmd.args([
        "wizard",
        "run",
        "--target",
        "flow",
        "--mode",
        "create",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    run_cmd.assert().success();

    let mut replay_cmd = cargo_bin_cmd!("greentic-dev");
    replay_cmd.args([
        "wizard",
        "replay",
        "--answers",
        answers.to_str().expect("utf8 path"),
    ]);
    replay_cmd
        .assert()
        .success()
        .stdout(contains("\"target\": \"flow\""))
        .stdout(contains("\"mode\": \"create\""));
}

#[test]
fn wizard_execute_in_non_interactive_requires_yes_or_non_interactive() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--execute",
    ]);
    cmd.assert().failure().stderr(contains(
        "refusing to execute in non-interactive mode without confirmation",
    ));
}

#[test]
fn wizard_dry_run_snapshot_matches() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-snapshot");

    let output = cargo_bin_cmd!("greentic-dev")
        .args([
            "wizard",
            "run",
            "--target",
            "pack",
            "--mode",
            "build",
            "--frontend",
            "json",
            "--locale",
            "en-US",
            "--out",
            out.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run wizard");
    assert!(
        output.status.success(),
        "wizard run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = String::from_utf8(output.stdout).expect("utf8 stdout");
    let expected =
        fs::read_to_string("tests/snapshots/wizard_pack_build_plan.json").expect("read snapshot");
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "wizard dry-run snapshot mismatch"
    );
}

#[test]
fn wizard_flow_create_snapshot_matches() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-flow-snapshot");
    let answers_file = tmp.path().join("answers-flow.json");
    fs::write(
        &answers_file,
        r#"{
  "id": "main",
  "out": "flows/main.ygtc",
  "provider_refs": {
    "flow": "flow://demo/main@1.0.0"
  }
}
"#,
    )
    .expect("write answers");

    let output = cargo_bin_cmd!("greentic-dev")
        .args([
            "wizard",
            "run",
            "--target",
            "flow",
            "--mode",
            "create",
            "--frontend",
            "json",
            "--locale",
            "en-US",
            "--answers",
            answers_file.to_str().expect("utf8 path"),
            "--out",
            out.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run wizard");
    assert!(
        output.status.success(),
        "wizard run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = String::from_utf8(output.stdout).expect("utf8 stdout");
    let expected =
        fs::read_to_string("tests/snapshots/wizard_flow_create_plan.json").expect("read snapshot");
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "wizard flow-create snapshot mismatch"
    );
}

#[test]
fn wizard_adaptive_card_frontend_outputs_card_json() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-card");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--frontend",
        "adaptive-card",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("\"type\": \"AdaptiveCard\""))
        .stdout(contains("\"plan_version\": 1"));
}

#[test]
fn wizard_replay_execute_blocks_disallowed_command() {
    let tmp = TempDir::new().expect("temp dir");
    let root = tmp.path().join("wiz-replay-disallowed");
    fs::create_dir_all(&root).expect("create replay root");
    let answers_path = root.join("answers.json");
    let plan_path = root.join("plan.json");

    fs::write(&answers_path, "{}").expect("write answers");
    fs::write(
        &plan_path,
        r#"{
  "plan_version": 1,
  "metadata": {
    "target": "dev",
    "mode": "run",
    "locale": "en-US",
    "frontend": "json"
  },
  "steps": [
    {
      "type": "RunCommand",
      "program": "bash",
      "args": ["--version"]
    }
  ]
}
"#,
    )
    .expect("write plan");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "replay",
        "--answers",
        answers_path.to_str().expect("utf8 path"),
        "--execute",
        "--non-interactive",
    ]);
    cmd.assert()
        .failure()
        .stderr(contains("is not allowed by default"));
}

#[test]
fn wizard_replay_execute_blocks_destructive_step_without_flag() {
    let tmp = TempDir::new().expect("temp dir");
    let root = tmp.path().join("wiz-replay-destructive");
    fs::create_dir_all(&root).expect("create replay root");
    let answers_path = root.join("answers.json");
    let plan_path = root.join("plan.json");

    fs::write(&answers_path, "{}").expect("write answers");
    fs::write(
        &plan_path,
        r#"{
  "plan_version": 1,
  "metadata": {
    "target": "pack",
    "mode": "build",
    "locale": "en-US",
    "frontend": "json"
  },
  "steps": [
    {
      "type": "RunCommand",
      "program": "greentic-pack",
      "args": ["build"],
      "destructive": true
    }
  ]
}
"#,
    )
    .expect("write plan");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "replay",
        "--answers",
        answers_path.to_str().expect("utf8 path"),
        "--execute",
        "--non-interactive",
    ]);
    cmd.assert()
        .failure()
        .stderr(contains("plan requested destructive operations"));
}

#[test]
fn wizard_execute_persists_resolved_versions_and_exec_log() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let runlog = tmp.path().join("stub-runs.log");
    let out = tmp.path().join("wiz-exec");

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
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--execute",
        "--non-interactive",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let plan = fs::read_to_string(out.join("plan.json")).expect("read plan");
    assert!(
        plan.contains("\"resolved_versions.greentic-pack\": \"greentic-pack 0.4.test\""),
        "plan should include resolved program version"
    );
    assert!(
        plan.contains("\"executed_commands\": \"1\""),
        "plan should record executed command count"
    );

    let exec_log = fs::read_to_string(out.join("exec.log")).expect("read exec log");
    assert!(
        exec_log.contains("RUN greentic-pack build"),
        "exec.log should include executed command"
    );

    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(
        stub_log.contains("build"),
        "stub should have been invoked with build args"
    );
}

#[test]
fn wizard_text_frontend_outputs_human_summary() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-text");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--frontend",
        "text",
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert()
        .success()
        .stdout(contains("wizard plan v1: pack.build"))
        .stdout(contains("1. ResolvePacks"))
        .stdout(contains("2. RunCommand greentic-pack build"));
}

#[test]
fn wizard_replay_execute_fails_when_pinned_version_mismatches_actual() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let root = tmp.path().join("wiz-replay-pin-mismatch");
    fs::create_dir_all(&root).expect("create replay root");
    let answers_path = root.join("answers.json");
    let plan_path = root.join("plan.json");

    write_stub_bin(
        &bin_dir,
        "greentic-pack",
        r#"
if [ "$1" = "--version" ]; then
  echo "greentic-pack 0.4.actual"
  exit 0
fi
exit 0
"#,
    );

    fs::write(&answers_path, "{}").expect("write answers");
    fs::write(
        &plan_path,
        r#"{
  "plan_version": 1,
  "metadata": {
    "target": "pack",
    "mode": "build",
    "locale": "en-US",
    "frontend": "json"
  },
  "inputs": {
    "resolved_versions.greentic-pack": "greentic-pack 0.4.pinned"
  },
  "steps": [
    {
      "type": "RunCommand",
      "program": "greentic-pack",
      "args": ["build"]
    }
  ]
}
"#,
    )
    .expect("write plan");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", prepend_path(&bin_dir)).args([
        "wizard",
        "replay",
        "--answers",
        answers_path.to_str().expect("utf8 path"),
        "--execute",
        "--non-interactive",
    ]);
    cmd.assert()
        .failure()
        .stderr(contains("replay pin mismatch for `greentic-pack`"));
}

#[test]
fn wizard_run_emit_answers_writes_answer_document_envelope() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-emit");
    let emitted = tmp.path().join("answers-envelope.json");

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args([
        "wizard",
        "run",
        "--target",
        "pack",
        "--mode",
        "build",
        "--emit-answers",
        emitted.to_str().expect("utf8 path"),
        "--out",
        out.to_str().expect("utf8 path"),
    ]);
    cmd.assert().success();

    let envelope = fs::read_to_string(&emitted).expect("read emitted envelope");
    assert!(envelope.contains("\"wizard_id\": \"greentic-dev.wizard.pack.build\""));
    assert!(envelope.contains("\"schema_id\": \"greentic-dev.pack.build\""));
    assert!(envelope.contains("\"schema_version\": \"1.0.0\""));
    assert!(envelope.contains("\"locale\": \"en-US\""));
    assert!(envelope.contains("\"answers\": {}"));
}

#[test]
fn wizard_validate_answers_document_runs_dry_run_plan() {
    let tmp = TempDir::new().expect("temp dir");
    let out = tmp.path().join("wiz-validate");
    let answers_doc = tmp.path().join("answers-doc.json");

    fs::write(
        &answers_doc,
        r#"{
  "wizard_id": "greentic-dev.wizard.pack.build",
  "schema_id": "greentic-dev.pack.build",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {
    "in": "."
  },
  "locks": {}
}
"#,
    )
    .expect("write answers doc");

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
        .stdout(contains("\"target\": \"pack\""))
        .stdout(contains("\"mode\": \"build\""));

    assert!(
        !out.join("exec.log").exists(),
        "dry-run validate should not create exec.log"
    );
}

#[test]
fn wizard_apply_answers_document_executes_plan() {
    let tmp = TempDir::new().expect("temp dir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let out = tmp.path().join("wiz-apply");
    let runlog = tmp.path().join("apply-runs.log");
    let answers_doc = tmp.path().join("answers-doc.json");

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

    fs::write(
        &answers_doc,
        r#"{
  "wizard_id": "greentic-dev.wizard.pack.build",
  "schema_id": "greentic-dev.pack.build",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {
    "in": "."
  },
  "locks": {}
}
"#,
    )
    .expect("write answers doc");

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
    assert!(exec_log.contains("RUN greentic-pack build --in ."));
    let stub_log = fs::read_to_string(&runlog).expect("read stub run log");
    assert!(stub_log.contains("build --in ."));
}
