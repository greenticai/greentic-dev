use anyhow::{Context, Result, bail};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cli::CoverageArgs;

const SUCCESS_EXIT_CODE: i32 = 0;
const POLICY_MISSING_EXIT_CODE: i32 = 2;
const SETUP_FAILURE_EXIT_CODE: i32 = 3;
const RUN_FAILURE_EXIT_CODE: i32 = 4;
const POLICY_FAILURE_EXIT_CODE: i32 = 5;

pub fn run(args: CoverageArgs) -> Result<()> {
    let exit_code = run_inner(args)?;
    if exit_code == SUCCESS_EXIT_CODE {
        return Ok(());
    }
    std::process::exit(exit_code);
}

fn run_inner(args: CoverageArgs) -> Result<i32> {
    let policy_file = PathBuf::from(
        std::env::var("COVERAGE_POLICY_FILE")
            .unwrap_or_else(|_| "coverage-policy.json".to_string()),
    );
    let report_dir = PathBuf::from(
        std::env::var("COVERAGE_REPORT_DIR").unwrap_or_else(|_| "target/coverage".to_string()),
    );
    let report_file = PathBuf::from(
        std::env::var("COVERAGE_REPORT_FILE")
            .unwrap_or_else(|_| report_dir.join("coverage.json").display().to_string()),
    );
    let offline = env_true("CARGO_NET_OFFLINE");

    if !policy_file.is_file() {
        print_policy_missing_instructions(&policy_file);
        return Ok(POLICY_MISSING_EXIT_CODE);
    }

    log("ensuring coverage tools are installed");
    if !args.skip_run {
        if let Err(err) = ensure_tool("cargo-llvm-cov", "cargo-llvm-cov", offline) {
            eprintln!("[coverage] {err}");
            return Ok(SETUP_FAILURE_EXIT_CODE);
        }
        if let Err(err) = ensure_tool("cargo-nextest", "cargo-nextest", offline) {
            eprintln!("[coverage] {err}");
            return Ok(SETUP_FAILURE_EXIT_CODE);
        }
        if let Err(err) = ensure_llvm_tools(offline) {
            eprintln!("[coverage] {err}");
            return Ok(SETUP_FAILURE_EXIT_CODE);
        }
    }

    fs::create_dir_all(&report_dir)
        .with_context(|| format!("failed to create {}", report_dir.display()))?;

    if args.skip_run {
        log(&format!(
            "skipping coverage run and reusing {}",
            report_file.display()
        ));
    } else {
        log("running cargo llvm-cov nextest");
        let status = Command::new("cargo")
            .args([
                "llvm-cov",
                "nextest",
                "--ignore-run-fail",
                "--json",
                "--output-path",
            ])
            .arg(&report_file)
            .args(["--workspace", "--all-features"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("failed to execute cargo llvm-cov nextest")?;
        if !status.success() {
            eprintln!("[coverage] coverage command failed before policy evaluation");
            return Ok(RUN_FAILURE_EXIT_CODE);
        }
    }

    if !report_file.is_file() {
        eprintln!(
            "[coverage] expected coverage report missing: {}",
            report_file.display()
        );
        return Ok(RUN_FAILURE_EXIT_CODE);
    }

    log(&format!("evaluating policy from {}", policy_file.display()));
    let policy = CoveragePolicy::load(&policy_file)?;
    let report = CoverageReport::load(&report_file)?;
    let result = evaluate_policy(&policy, &report, &std::env::current_dir()?);
    if !result.violations.is_empty() {
        println!("[coverage] policy check failed");
        println!("[coverage] Codex instructions:");
        println!(
            "Increase test coverage for the files below or update the exclusion list only for generated code, tooling entrypoints, or thin wiring layers."
        );
        println!(
            "Do not lower thresholds to make the report pass unless the team intentionally changes the policy."
        );
        println!("[coverage] violations:");
        for violation in result.violations {
            println!("- {violation}");
        }
        return Ok(POLICY_FAILURE_EXIT_CODE);
    }

    println!("[coverage] policy check passed");
    println!(
        "[coverage] workspace line coverage: {:.2}%",
        result.workspace_line_percent
    );
    log("success");
    log(&format!("report written to {}", report_file.display()));
    Ok(SUCCESS_EXIT_CODE)
}

fn log(message: &str) {
    println!("[coverage] {message}");
}

fn print_policy_missing_instructions(policy_file: &Path) {
    println!("[coverage] missing policy file: {}", policy_file.display());
    println!("[coverage] Codex instructions:");
    println!("Create coverage-policy.json at the repository root with:");
    println!("- a global line coverage minimum");
    println!("- a default per-file line coverage minimum");
    println!("- an explicit exclusion list for generated code or thin entrypoints");
    println!("- per-file overrides for high-risk modules that need stricter targets");
    println!("Suggested starting point:");
    println!("{{");
    println!("  \"version\": 1,");
    println!("  \"global\": {{ \"line_coverage_min\": 60.0 }},");
    println!("  \"defaults\": {{ \"per_file_line_coverage_min\": 60.0 }},");
    println!("  \"exclusions\": {{ \"files\": [] }},");
    println!("  \"per_file\": {{}}");
    println!("}}");
}

fn env_true(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn command_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

fn cargo_args_for_network(offline: bool) -> Vec<&'static str> {
    if offline {
        Vec::new()
    } else {
        vec!["--locked"]
    }
}

fn ensure_binstall(offline: bool) -> Result<()> {
    if command_exists("cargo-binstall") {
        return Ok(());
    }
    if offline {
        bail!("cargo-binstall is required but offline mode is enabled");
    }

    log("installing cargo-binstall");
    let mut args = vec!["install", "cargo-binstall"];
    args.extend(cargo_args_for_network(offline));
    let status = Command::new("cargo")
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to install cargo-binstall")?;
    if !status.success() {
        bail!("failed to install cargo-binstall");
    }
    Ok(())
}

fn ensure_tool(bin: &str, package: &str, offline: bool) -> Result<()> {
    if command_exists(bin) {
        return Ok(());
    }
    ensure_binstall(offline)?;
    if offline {
        bail!("missing {package} but offline mode is enabled");
    }

    log(&format!("installing {package}"));
    let mut command = Command::new("cargo");
    command.arg("binstall");
    command.args(cargo_args_for_network(offline));
    command.args(["-y", package]);
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to install {package}"))?;
    if !status.success() {
        bail!("failed to install {package}");
    }
    Ok(())
}

fn ensure_llvm_tools(offline: bool) -> Result<()> {
    if !command_exists("rustup") {
        bail!("rustup is required to add llvm-tools-preview");
    }

    let output = Command::new("rustup")
        .args(["component", "list", "--installed"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("failed to inspect rustup components")?;
    let stdout = String::from_utf8(output.stdout).context("rustup output was not valid UTF-8")?;
    if stdout
        .lines()
        .any(|line| line.trim() == "llvm-tools-preview")
    {
        return Ok(());
    }

    if offline {
        bail!("llvm-tools-preview is missing and offline mode is enabled");
    }

    log("installing llvm-tools-preview");
    let status = Command::new("rustup")
        .args(["component", "add", "llvm-tools-preview"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to install llvm-tools-preview")?;
    if !status.success() {
        bail!("failed to install llvm-tools-preview");
    }
    Ok(())
}

#[derive(Debug)]
struct CoveragePolicy {
    global_line_min: f64,
    default_per_file_min: f64,
    excluded_paths: BTreeSet<String>,
    per_file_line_min: BTreeMap<String, f64>,
}

impl CoveragePolicy {
    fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let json: JsonValue = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let global_line_min = json
            .get("global")
            .and_then(|v| v.get("line_coverage_min"))
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let default_per_file_min = json
            .get("defaults")
            .and_then(|v| v.get("per_file_line_coverage_min"))
            .and_then(JsonValue::as_f64)
            .unwrap_or(global_line_min);

        let mut excluded_paths = BTreeSet::new();
        if let Some(files) = json
            .get("exclusions")
            .and_then(|v| v.get("files"))
            .and_then(JsonValue::as_array)
        {
            for entry in files {
                match entry {
                    JsonValue::String(path) => {
                        excluded_paths.insert(path.clone());
                    }
                    JsonValue::Object(map) => {
                        if let Some(path) = map.get("path").and_then(JsonValue::as_str) {
                            excluded_paths.insert(path.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut per_file_line_min = BTreeMap::new();
        if let Some(per_file) = json.get("per_file").and_then(JsonValue::as_object) {
            for (path, cfg) in per_file {
                if let Some(min) = cfg.get("line_coverage_min").and_then(JsonValue::as_f64) {
                    per_file_line_min.insert(path.clone(), min);
                }
            }
        }

        Ok(Self {
            global_line_min,
            default_per_file_min,
            excluded_paths,
            per_file_line_min,
        })
    }
}

#[derive(Debug)]
struct CoverageReport {
    files: Vec<FileCoverage>,
    total_line_percent: f64,
}

#[derive(Debug)]
struct FileCoverage {
    rel_path: String,
    line_percent: f64,
    line_count: u64,
    line_covered: u64,
}

impl CoverageReport {
    fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let json: JsonValue = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        let root = std::env::current_dir()?;
        let data0 = json
            .get("data")
            .and_then(JsonValue::as_array)
            .and_then(|arr| arr.first())
            .cloned()
            .unwrap_or_else(|| json.clone());

        let total_line_percent = data0
            .get("totals")
            .and_then(|v| v.get("lines"))
            .and_then(|v| v.get("percent"))
            .and_then(JsonValue::as_f64)
            .or_else(|| {
                json.get("totals")
                    .and_then(|v| v.get("lines"))
                    .and_then(|v| v.get("percent"))
                    .and_then(JsonValue::as_f64)
            })
            .unwrap_or(0.0);

        let files_json = data0
            .get("files")
            .and_then(JsonValue::as_array)
            .or_else(|| json.get("files").and_then(JsonValue::as_array))
            .cloned()
            .unwrap_or_default();

        let mut files = Vec::new();
        for file in files_json {
            let Some(filename) = file.get("filename").and_then(JsonValue::as_str) else {
                continue;
            };
            let rel_path = relativize_path(&root, filename);
            let line_summary = file
                .get("summary")
                .and_then(|v| v.get("lines"))
                .cloned()
                .unwrap_or(JsonValue::Null);
            files.push(FileCoverage {
                rel_path,
                line_percent: line_summary
                    .get("percent")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or(0.0),
                line_count: line_summary
                    .get("count")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0),
                line_covered: line_summary
                    .get("covered")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0),
            });
        }

        Ok(Self {
            files,
            total_line_percent,
        })
    }
}

fn relativize_path(root: &Path, raw: &str) -> String {
    let path = PathBuf::from(raw);
    path.canonicalize()
        .ok()
        .and_then(|canon| {
            canon
                .strip_prefix(root)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_else(|| raw.replace('\\', "/"))
}

#[derive(Debug)]
struct PolicyEvaluation {
    workspace_line_percent: f64,
    violations: Vec<String>,
}

fn evaluate_policy(
    policy: &CoveragePolicy,
    report: &CoverageReport,
    _repo_root: &Path,
) -> PolicyEvaluation {
    let mut effective_line_count = 0u64;
    let mut effective_line_covered = 0u64;
    let mut violations = Vec::new();

    for file in &report.files {
        if policy.excluded_paths.contains(&file.rel_path) {
            continue;
        }

        effective_line_count += file.line_count;
        effective_line_covered += file.line_covered;
        let expected = policy
            .per_file_line_min
            .get(&file.rel_path)
            .copied()
            .unwrap_or(policy.default_per_file_min);
        if file.line_percent < expected {
            violations.push(format!(
                "{} line coverage {:.2}% is below required minimum {:.2}%",
                file.rel_path, file.line_percent, expected
            ));
        }
    }

    let workspace_line_percent = if effective_line_count == 0 {
        report.total_line_percent
    } else {
        (effective_line_covered as f64 / effective_line_count as f64) * 100.0
    };

    if workspace_line_percent < policy.global_line_min {
        violations.insert(
            0,
            format!(
                "workspace line coverage {:.2}% is below global minimum {:.2}%",
                workspace_line_percent, policy.global_line_min
            ),
        );
    }

    PolicyEvaluation {
        workspace_line_percent,
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::{CoveragePolicy, CoverageReport, evaluate_policy, relativize_path};
    use std::collections::BTreeMap;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn relativize_path_prefers_repo_relative_paths() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("src").join("demo.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let rel = relativize_path(dir.path(), file.to_str().unwrap());
        assert_eq!(rel, "src/demo.rs");
    }

    #[test]
    fn policy_loader_supports_exclusions_and_overrides() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("coverage-policy.json");
        std::fs::write(
            &path,
            r#"{
              "global": { "line_coverage_min": 60.0 },
              "defaults": { "per_file_line_coverage_min": 55.0 },
              "exclusions": { "files": [ { "path": "src/generated.rs" }, "src/wrapper.rs" ] },
              "per_file": { "src/core.rs": { "line_coverage_min": 80.0 } }
            }"#,
        )
        .unwrap();

        let policy = CoveragePolicy::load(&path).unwrap();
        assert_eq!(policy.global_line_min, 60.0);
        assert_eq!(policy.default_per_file_min, 55.0);
        assert!(policy.excluded_paths.contains("src/generated.rs"));
        assert_eq!(policy.per_file_line_min["src/core.rs"], 80.0);
    }

    #[test]
    fn report_loader_reads_llvm_cov_json_shape() {
        let dir = tempdir().unwrap();
        let report_path = dir.path().join("coverage.json");
        let file = dir.path().join("src").join("demo.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn demo() {}\n").unwrap();

        std::fs::write(
            &report_path,
            format!(
                r#"{{
                  "data": [{{
                    "totals": {{ "lines": {{ "percent": 50.0 }} }},
                    "files": [{{
                      "filename": "{}",
                      "summary": {{ "lines": {{ "percent": 75.0, "count": 4, "covered": 3 }} }}
                    }}]
                  }}]
                }}"#,
                file.display()
            ),
        )
        .unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let report = CoverageReport::load(&report_path).unwrap();
        std::env::set_current_dir(old_cwd).unwrap();

        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].rel_path, "src/demo.rs");
        assert_eq!(report.files[0].line_percent, 75.0);
    }

    #[test]
    fn evaluation_uses_excluded_files_for_neither_global_nor_per_file_checks() {
        let report = CoverageReport {
            total_line_percent: 10.0,
            files: vec![
                super::FileCoverage {
                    rel_path: "src/generated.rs".to_string(),
                    line_percent: 0.0,
                    line_count: 100,
                    line_covered: 0,
                },
                super::FileCoverage {
                    rel_path: "src/core.rs".to_string(),
                    line_percent: 75.0,
                    line_count: 4,
                    line_covered: 3,
                },
            ],
        };
        let policy = CoveragePolicy {
            global_line_min: 60.0,
            default_per_file_min: 60.0,
            excluded_paths: ["src/generated.rs".to_string()].into_iter().collect(),
            per_file_line_min: BTreeMap::new(),
        };

        let result = evaluate_policy(&policy, &report, Path::new("."));
        assert!(result.violations.is_empty());
        assert_eq!(format!("{:.2}", result.workspace_line_percent), "75.00");
    }
}
