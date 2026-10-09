//! End-to-end tests for `greentic-dev mcp gen` passthrough and the
//! toolmap-less `mcp doctor` generator/toolchain report.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

/// Write an executable shell stub into `dir` and return its path.
fn write_stub(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, script).expect("write stub");
    let mut perms = fs::metadata(&path).expect("stat stub").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod stub");
    path
}

#[test]
fn mcp_gen_forwards_args_and_propagates_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stub = write_stub(
        dir.path(),
        "greentic-mcp-gen",
        "#!/bin/sh\necho \"ARGS: $*\"\nexit 7\n",
    );

    Command::cargo_bin("greentic-dev")
        .expect("bin")
        .env("GREENTIC_DEV_BIN_GREENTIC_MCP_GEN", &stub)
        .args(["mcp", "gen", "--spec", "api.yaml", "discovery"])
        .assert()
        .code(7)
        .stdout(predicates::str::contains("ARGS: --spec api.yaml discovery"));
}

#[test]
fn mcp_gen_missing_binary_reports_guided_error() {
    Command::cargo_bin("greentic-dev")
        .expect("bin")
        .env(
            "GREENTIC_DEV_BIN_GREENTIC_MCP_GEN",
            "/nonexistent/greentic-mcp-gen",
        )
        .args(["mcp", "gen", "--spec", "api.yaml"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("greentic-mcp-gen"));
}

#[test]
fn mcp_doctor_without_toolmap_succeeds() {
    Command::cargo_bin("greentic-dev")
        .expect("bin")
        .env(
            "GREENTIC_DEV_BIN_GREENTIC_MCP_GEN",
            "/nonexistent/greentic-mcp-gen",
        )
        .args(["mcp", "doctor"])
        .assert()
        .success();
}

#[test]
fn mcp_doctor_without_toolmap_json_reports_generator() {
    let output = Command::cargo_bin("greentic-dev")
        .expect("bin")
        .env(
            "GREENTIC_DEV_BIN_GREENTIC_MCP_GEN",
            "/nonexistent/greentic-mcp-gen",
        )
        .args(["mcp", "doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value =
        serde_json::from_slice(&output).expect("doctor --json must emit valid JSON");
    assert_eq!(value["binary_name"], "greentic-mcp-gen");
}
