use assert_cmd::cargo::{cargo_bin, cargo_bin_cmd};
use predicates::str::contains;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn dev_binary_path(dir: &Path) -> PathBuf {
    let source = cargo_bin("greentic-dev");
    #[cfg(windows)]
    {
        let target = dir.join("greentic-dev-dev.exe");
        fs::copy(&source, &target).expect("copy greentic-dev binary");
        return target;
    }

    #[cfg(not(windows))]
    {
        let target = dir.join("greentic-dev-dev");
        fs::copy(&source, &target).expect("copy greentic-dev binary");
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target, perms).expect("set mode");
        target
    }
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
fn dev_binary_unknown_subcommand_delegates_to_dev_prefixed_binary() {
    let bin_dir = TempDir::new().expect("tempdir");
    let exe_dir = TempDir::new().expect("tempdir");
    let dev_exe = dev_binary_path(exe_dir.path());
    let _stable_stub = write_script(
        bin_dir.path(),
        "greentic-foo",
        r#"echo "stable:$1:$2"; exit 24"#,
    );
    let _dev_stub = write_script(
        bin_dir.path(),
        "greentic-foo-dev",
        r#"echo "dev:$1:$2"; exit 23"#,
    );

    // Accepted risk: test-only execution of a temporary greentic-dev copy with controlled PATH fixtures.
    // foxguard: ignore[rs/no-command-injection]
    let mut cmd = Command::new(dev_exe);
    cmd.env("PATH", path_with(bin_dir.path()));
    cmd.args(["foo", "bar", "--baz=1"]);

    let output = cmd.output().expect("run greentic-dev-dev copy");
    assert_eq!(output.status.code(), Some(23));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("dev:bar:--baz=1"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn dev_binary_known_passthrough_uses_dev_tool_binary() {
    let bin_dir = TempDir::new().expect("tempdir");
    let exe_dir = TempDir::new().expect("tempdir");
    let dev_exe = dev_binary_path(exe_dir.path());
    let _stable_stub = write_script(
        bin_dir.path(),
        "greentic-pack",
        r#"echo "stable-pack:$1"; exit 24"#,
    );
    let _dev_stub = write_script(
        bin_dir.path(),
        "greentic-pack-dev",
        r#"echo "dev-pack:$1"; exit 0"#,
    );

    // Accepted risk: test-only execution of a temporary greentic-dev copy with controlled PATH fixtures.
    // foxguard: ignore[rs/no-command-injection]
    let mut cmd = Command::new(dev_exe);
    cmd.env("PATH", path_with(bin_dir.path()));
    cmd.args(["pack", "doctor"]);

    let output = cmd.output().expect("run greentic-dev-dev copy");
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("dev-pack:doctor"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
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
