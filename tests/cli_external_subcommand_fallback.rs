use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn path_with(bin_dir: &Path) -> OsString {
    use std::env;

    let mut value = OsString::from(bin_dir.as_os_str());
    if let Some(existing) = env::var_os("PATH") {
        value.push(if cfg!(windows) { ";" } else { ":" });
        value.push(existing);
    }
    value
}

#[cfg(windows)]
fn write_script(dir: &Path, name: &str, script_body: &str) -> PathBuf {
    let path = dir.join(format!("{name}.cmd"));
    let script = format!("@echo off\r\n{script_body}\r\n");
    fs::write(&path, script).expect("write script");
    path
}

#[cfg(not(windows))]
fn write_script(dir: &Path, name: &str, script_body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    let script = format!("#!/bin/sh\n{script_body}\n");
    fs::write(&path, script).expect("write script");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("set mode");
    path
}

#[test]
fn unknown_subcommand_delegates_to_greentic_prefixed_binary() {
    let bin_dir = TempDir::new().expect("tempdir");
    let _stub = write_script(
        bin_dir.path(),
        "greentic-foo",
        r#"echo "delegated:$1:$2"; exit 23"#,
    );

    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("PATH", path_with(bin_dir.path()));
    cmd.args(["foo", "bar", "--baz=1"]);

    cmd.assert()
        .code(23)
        .stdout(contains("delegated:bar:--baz=1"));
}

#[test]
fn unknown_subcommand_without_binary_falls_back_to_clap_error() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["flo"]);

    cmd.assert()
        .failure()
        .stderr(contains("unrecognized subcommand"))
        .stderr(contains("flo"))
        .stderr(contains("flow"));
}
