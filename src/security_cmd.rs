use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_yaml_bw::{Mapping as YamlMapping, Sequence as YamlSequence, Value as YamlValue};

use crate::cli::{SecurityArgs, SecurityFormat};

const EXIT_SUCCESS: i32 = 0;
const EXIT_POLICY_VIOLATION: i32 = 2;
const EXIT_OPERATIONAL_ERROR: i32 = 3;
const FOXGUARD_REPORT_PATH: &str = "target/greentic-security/foxguard-report.md";
const FOXGUARD_CONFIG_PATH: &str = ".foxguard.yml";
const DEFAULT_FOXGUARD_EXCLUDE_DIRS: &[&str] = &[
    "tests",
    "benches",
    "docs",
    "examples",
    "fixtures",
    "testdata",
    "generated",
];
const DEFAULT_FOXGUARD_IGNORE_RULES: &[&str] = &[
    "rs/no-command-injection",
    "rs/no-path-traversal",
    "rs/no-unwrap-in-lib",
    "rs/unsafe-block",
];

pub fn run(args: SecurityArgs) -> i32 {
    match run_inner(&ShellRunner, args) {
        Ok(outcome) => {
            eprintln!("{}", outcome.policy_notice);
            println!("{}", outcome.output.trim_end());
            outcome.exit_code
        }
        Err(err) => {
            eprintln!("{err}");
            EXIT_OPERATIONAL_ERROR
        }
    }
}

fn run_inner(runner: &dyn Runner, args: SecurityArgs) -> Result<RunOutcome, OperationalError> {
    let options = QueryOptions::from_args(&args)?;
    let repository = match args.repo.as_deref() {
        Some(raw) => parse_owner_repo(raw)?,
        None => {
            let remote = runner.git(&["remote", "get-url", "origin"])?;
            parse_github_remote(remote.trim())?
        }
    };
    let branch = match args.branch.as_deref() {
        Some(branch) => branch.to_string(),
        None => {
            let branch = runner.git(&["branch", "--show-current"])?;
            let branch = branch.trim();
            if branch.is_empty() {
                return Err(OperationalError::new(
                    "detached HEAD detected; pass --branch BRANCH to query code scanning alerts",
                ));
            }
            branch.to_string()
        }
    };
    let commit = runner.git(&["rev-parse", "HEAD"])?.trim().to_string();

    let mut alerts = Vec::new();
    let mut warnings = Vec::new();
    match fetch_code_scanning_alerts(runner, &repository, &branch, &options) {
        Ok(source_alerts) => alerts.extend(source_alerts),
        Err(err) => warnings.push(format!("code_scanning skipped: {err}")),
    }
    match fetch_dependabot_alerts(runner, &repository, &options) {
        Ok(source_alerts) => alerts.extend(source_alerts),
        Err(err) => warnings.push(format!("dependabot skipped: {err}")),
    }
    match fetch_secret_scanning_alerts(runner, &repository, &options) {
        Ok(source_alerts) => alerts.extend(source_alerts),
        Err(err) => warnings.push(format!("secret_scanning skipped: {err}")),
    }
    match runner.ensure_foxguard_config() {
        Ok(Some(message)) => warnings.push(message),
        Ok(None) => {}
        Err(err) => warnings.push(format!("foxguard config update skipped: {err}")),
    }
    match fetch_foxguard_alerts(runner) {
        Ok(source_alerts) => alerts.extend(source_alerts),
        Err(err) => warnings.push(format!("foxguard skipped: {err}")),
    }

    let alerts = alerts
        .into_iter()
        .filter(|alert| options.matches(alert))
        .collect::<Vec<_>>();
    let foxguard_report_path = write_foxguard_report(runner, &alerts)?;
    let report = Report::new(
        ReportMetadata {
            repository: repository.to_string(),
            branch,
            commit,
            state_filter: options.states.clone(),
            severity_filter: options.severities.clone(),
            security_severity_filter: options.security_severities.clone(),
            warnings,
            foxguard_report_path,
        },
        alerts,
    );
    let output = if args.prompt {
        render_prompt(&report)
    } else {
        match args.format {
            SecurityFormat::Json => serde_json::to_string_pretty(&report).map_err(|err| {
                OperationalError::new(format!("failed to render JSON report: {err}"))
            })?,
            SecurityFormat::Markdown => render_markdown(&report),
        }
    };
    let exit_code = if args.no_errors || !report.policy.has_blocking_issues {
        EXIT_SUCCESS
    } else {
        EXIT_POLICY_VIOLATION
    };

    let policy_notice = render_policy_notice(&report.policy, args.no_errors);

    Ok(RunOutcome {
        output,
        exit_code,
        policy_notice,
    })
}

fn fetch_code_scanning_alerts(
    runner: &dyn Runner,
    repository: &OwnerRepo,
    branch: &str,
    options: &QueryOptions,
) -> Result<Vec<Alert>, OperationalError> {
    let mut alerts = Vec::new();
    for state in options.states_for(AlertSource::CodeScanning) {
        let endpoint = format!(
            "/repos/{}/code-scanning/alerts?state={}&ref=refs/heads/{}",
            repository, state, branch
        );
        let body = match runner.gh_api(&endpoint) {
            Ok(body) => body,
            Err(err) if err.is_feature_unavailable() => return Ok(alerts),
            Err(err) => return Err(err),
        };
        let api_alerts: Vec<CodeScanningAlert> = serde_json::from_str(&body).map_err(|err| {
            OperationalError::new(format!("failed to parse code scanning API response: {err}"))
        })?;
        alerts.extend(api_alerts.into_iter().map(normalize_code_scanning_alert));
    }
    Ok(alerts)
}

fn fetch_dependabot_alerts(
    runner: &dyn Runner,
    repository: &OwnerRepo,
    options: &QueryOptions,
) -> Result<Vec<Alert>, OperationalError> {
    let mut alerts = Vec::new();
    for state in options.states_for(AlertSource::Dependabot) {
        let endpoint = format!("/repos/{}/dependabot/alerts?state={}", repository, state);
        let body = match runner.gh_api(&endpoint) {
            Ok(body) => body,
            Err(err) if err.is_feature_unavailable() => return Ok(alerts),
            Err(err) => return Err(err),
        };
        let api_alerts: Vec<DependabotAlert> = serde_json::from_str(&body).map_err(|err| {
            OperationalError::new(format!("failed to parse Dependabot API response: {err}"))
        })?;
        alerts.extend(api_alerts.into_iter().map(normalize_dependabot_alert));
    }
    Ok(alerts)
}

fn fetch_secret_scanning_alerts(
    runner: &dyn Runner,
    repository: &OwnerRepo,
    options: &QueryOptions,
) -> Result<Vec<Alert>, OperationalError> {
    let mut alerts = Vec::new();
    for state in options.states_for(AlertSource::SecretScanning) {
        let endpoint = format!(
            "/repos/{}/secret-scanning/alerts?state={}",
            repository, state
        );
        let body = match runner.gh_api(&endpoint) {
            Ok(body) => body,
            Err(err) if err.is_feature_unavailable() => return Ok(alerts),
            Err(err) => return Err(err),
        };
        let api_alerts: Vec<SecretScanningAlert> = serde_json::from_str(&body).map_err(|err| {
            OperationalError::new(format!(
                "failed to parse secret scanning API response: {err}"
            ))
        })?;
        alerts.extend(api_alerts.into_iter().map(normalize_secret_scanning_alert));
    }
    Ok(alerts)
}

fn fetch_foxguard_alerts(runner: &dyn Runner) -> Result<Vec<Alert>, OperationalError> {
    let mut alerts = Vec::new();
    let scan_args = foxguard_scan_args();
    let scan = match runner.local_command("foxguard", &scan_args) {
        Ok(output) => output,
        Err(err) if err.is_feature_unavailable() => return Ok(alerts),
        Err(err) => return Err(err),
    };
    alerts.extend(parse_foxguard_findings(&scan, "scan")?);

    let secrets = match runner.local_command("foxguard", &["secrets", "--format", "json", "."]) {
        Ok(output) => output,
        Err(err) if err.is_feature_unavailable() => return Ok(alerts),
        Err(err) => return Err(err),
    };
    alerts.extend(parse_foxguard_findings(&secrets, "secrets")?);
    Ok(alerts)
}

fn foxguard_scan_args() -> Vec<&'static str> {
    let mut args = vec![".", "--format", "json"];
    for dir in DEFAULT_FOXGUARD_EXCLUDE_DIRS {
        args.push("--exclude");
        args.push(dir);
    }
    args
}

fn parse_foxguard_findings(raw: &str, mode: &str) -> Result<Vec<Alert>, OperationalError> {
    let findings: Vec<FoxguardFinding> = serde_json::from_str(raw).map_err(|err| {
        OperationalError::new(format!(
            "failed to parse foxguard {mode} JSON response: {err}"
        ))
    })?;
    Ok(findings
        .into_iter()
        .map(|finding| normalize_foxguard_finding(finding, mode))
        .collect())
}

fn write_foxguard_report(
    runner: &dyn Runner,
    alerts: &[Alert],
) -> Result<Option<String>, OperationalError> {
    let foxguard_alerts = alerts
        .iter()
        .filter(|alert| alert.source == AlertSource::Foxguard)
        .cloned()
        .collect::<Vec<_>>();
    if foxguard_alerts.is_empty() {
        return Ok(None);
    }
    let content = render_foxguard_report(&foxguard_alerts);
    runner
        .write_report_file(FOXGUARD_REPORT_PATH, &content)
        .map(Some)
}

struct RunOutcome {
    output: String,
    exit_code: i32,
    policy_notice: String,
}

trait Runner {
    fn git(&self, args: &[&str]) -> Result<String, OperationalError>;
    fn gh_api(&self, endpoint: &str) -> Result<String, OperationalError>;
    fn ensure_foxguard_config(&self) -> Result<Option<String>, OperationalError>;
    fn write_report_file(&self, path: &str, content: &str) -> Result<String, OperationalError>;
    fn local_command(&self, program: &str, args: &[&str]) -> Result<String, OperationalError>;
}

struct ShellRunner;

impl Runner for ShellRunner {
    fn git(&self, args: &[&str]) -> Result<String, OperationalError> {
        run_command("git", args, "git")
    }

    fn gh_api(&self, endpoint: &str) -> Result<String, OperationalError> {
        run_command("gh", &["api", endpoint], "GitHub CLI (gh)")
    }

    fn ensure_foxguard_config(&self) -> Result<Option<String>, OperationalError> {
        ensure_default_foxguard_config_at(Path::new(FOXGUARD_CONFIG_PATH))
    }

    fn write_report_file(&self, path: &str, content: &str) -> Result<String, OperationalError> {
        let path = Path::new(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OperationalError::new(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        fs::write(path, content).map_err(|err| {
            OperationalError::new(format!("failed to write {}: {err}", path.display()))
        })?;
        Ok(path.display().to_string())
    }

    fn local_command(&self, program: &str, args: &[&str]) -> Result<String, OperationalError> {
        match run_command_accepting(program, args, program, &[0, 1]) {
            Ok(output) => Ok(output),
            Err(err) if program == "foxguard" && err.is_missing_foxguard() => {
                install_foxguard()?;
                run_command_accepting(program, args, program, &[0, 1]).or_else(|retry_err| {
                    if retry_err.is_missing_foxguard() {
                        run_foxguard_with_cargo_bin(args, &[0, 1])
                    } else {
                        Err(retry_err)
                    }
                })
            }
            Err(err) => Err(err),
        }
    }
}

fn install_foxguard() -> Result<(), OperationalError> {
    let output = Command::new("cargo")
        .args(["binstall", "--maximum-resolution-timeout", "60", "foxguard"])
        .output()
        .map_err(|err| OperationalError::new(format!("failed to execute cargo binstall: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(OperationalError::new(format!(
        "failed to install foxguard with `cargo binstall foxguard`.{}",
        suffix_stderr(&stderr)
    )))
}

fn foxguard_cargo_bin_dir() -> Result<OsString, OperationalError> {
    let Some(home) = dirs::home_dir() else {
        return Err(OperationalError::new(
            "foxguard was installed but the home directory could not be resolved",
        ));
    };
    Ok(home.join(".cargo").join("bin").into_os_string())
}

fn ensure_default_foxguard_config_at(path: &Path) -> Result<Option<String>, OperationalError> {
    let ignore_paths = default_foxguard_config_paths();

    if !path.exists() {
        fs::write(path, default_foxguard_config(&ignore_paths)).map_err(|err| {
            OperationalError::new(format!("failed to write {}: {err}", path.display()))
        })?;
        return Ok(Some(format!(
            "foxguard config created: {} with default non-distributed path ignores",
            path.display()
        )));
    }

    let raw = fs::read_to_string(path).map_err(|err| {
        OperationalError::new(format!("failed to read {}: {err}", path.display()))
    })?;
    let mut document = if raw.trim().is_empty() {
        YamlValue::Mapping(YamlMapping::new())
    } else {
        serde_yaml_bw::from_str::<YamlValue>(&raw).map_err(|err| {
            OperationalError::new(format!("failed to parse {}: {err}", path.display()))
        })?
    };
    let changed = merge_default_foxguard_ignores(&mut document, &ignore_paths)?;
    if !changed {
        return Ok(None);
    }
    let rendered = serde_yaml_bw::to_string(&document).map_err(|err| {
        OperationalError::new(format!(
            "failed to render updated {}: {err}",
            path.display()
        ))
    })?;
    fs::write(path, rendered).map_err(|err| {
        OperationalError::new(format!("failed to write {}: {err}", path.display()))
    })?;
    Ok(Some(format!(
        "foxguard config updated: {} with default non-distributed path ignores",
        path.display()
    )))
}

fn default_foxguard_config_paths() -> Vec<String> {
    DEFAULT_FOXGUARD_EXCLUDE_DIRS
        .iter()
        .map(|dir| format!("{dir}/"))
        .collect()
}

fn default_foxguard_config(ignore_paths: &[String]) -> String {
    render_foxguard_config(ignore_paths, DEFAULT_FOXGUARD_IGNORE_RULES)
}

fn render_foxguard_config(paths: &[String], rules: &[&str]) -> String {
    let mut out = String::from("scan:\n  ignore_rules:\n");
    for path in paths {
        out.push_str(&format!("    - path: {path}\n"));
        out.push_str("      rules:\n");
        for rule in rules {
            out.push_str(&format!("        - {rule}\n"));
        }
    }
    out
}

fn merge_default_foxguard_ignores(
    document: &mut YamlValue,
    ignore_paths: &[String],
) -> Result<bool, OperationalError> {
    let root = ensure_yaml_mapping(document, "root")?;
    let scan = ensure_child_mapping(root, "scan")?;
    let ignore_rules = ensure_child_sequence(scan, "ignore_rules")?;
    let mut changed = false;
    for path in ignore_paths {
        changed |= ensure_foxguard_ignore_entry(ignore_rules, path, DEFAULT_FOXGUARD_IGNORE_RULES)?;
    }
    Ok(changed)
}

fn ensure_yaml_mapping<'a>(
    value: &'a mut YamlValue,
    label: &str,
) -> Result<&'a mut YamlMapping, OperationalError> {
    match value {
        YamlValue::Mapping(mapping) => Ok(mapping),
        YamlValue::Null(_) => {
            *value = YamlValue::Mapping(YamlMapping::new());
            match value {
                YamlValue::Mapping(mapping) => Ok(mapping),
                _ => unreachable!("value was just replaced with a mapping"),
            }
        }
        _ => Err(OperationalError::new(format!(
            ".foxguard.yml {label} must be a YAML mapping"
        ))),
    }
}

fn ensure_child_mapping<'a>(
    parent: &'a mut YamlMapping,
    key: &str,
) -> Result<&'a mut YamlMapping, OperationalError> {
    let key_value = yaml_string_value(key);
    if !parent.contains_key(&key_value) {
        parent.insert(key_value.clone(), YamlValue::Mapping(YamlMapping::new()));
    }
    let Some(value) = parent.get_mut(&key_value) else {
        unreachable!("key was just inserted if missing");
    };
    ensure_yaml_mapping(value, key)
}

fn ensure_child_sequence<'a>(
    parent: &'a mut YamlMapping,
    key: &str,
) -> Result<&'a mut Vec<YamlValue>, OperationalError> {
    let key_value = yaml_string_value(key);
    if !parent.contains_key(&key_value) {
        parent.insert(key_value.clone(), YamlValue::Sequence(YamlSequence::new()));
    }
    let Some(value) = parent.get_mut(&key_value) else {
        unreachable!("key was just inserted if missing");
    };
    match value {
        YamlValue::Sequence(sequence) => Ok(sequence),
        _ => Err(OperationalError::new(format!(
            ".foxguard.yml {key} must be a YAML list"
        ))),
    }
}

fn ensure_foxguard_ignore_entry(
    entries: &mut Vec<YamlValue>,
    path: &str,
    rules: &[&str],
) -> Result<bool, OperationalError> {
    for entry in entries.iter_mut() {
        let mapping = ensure_yaml_mapping(entry, "scan.ignore_rules entry")?;
        if yaml_string(mapping, "path") == Some(path) {
            return merge_rule_list(mapping, rules);
        }
    }

    let mut mapping = YamlMapping::new();
    mapping.insert(yaml_string_value("path"), yaml_string_value(path));
    mapping.insert(
        yaml_string_value("rules"),
        YamlValue::Sequence(YamlSequence {
            anchor: None,
            elements: rules.iter().map(|rule| yaml_string_value(rule)).collect(),
        }),
    );
    entries.push(YamlValue::Mapping(mapping));
    Ok(true)
}

fn merge_rule_list(mapping: &mut YamlMapping, rules: &[&str]) -> Result<bool, OperationalError> {
    let sequence = ensure_child_sequence(mapping, "rules")?;
    let mut existing = sequence
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<BTreeSet<_>>();
    let mut changed = false;
    for rule in rules {
        if existing.insert((*rule).to_string()) {
            sequence.push(yaml_string_value(rule));
            changed = true;
        }
    }
    Ok(changed)
}

fn yaml_string<'a>(mapping: &'a YamlMapping, key: &str) -> Option<&'a str> {
    mapping
        .get(yaml_string_value(key))
        .and_then(YamlValue::as_str)
}

fn yaml_string_value(value: &str) -> YamlValue {
    YamlValue::String(value.to_string(), None)
}

fn run_command(program: &str, args: &[&str], label: &str) -> Result<String, OperationalError> {
    run_command_accepting(program, args, label, &[0])
}

fn run_command_accepting(
    program: &str,
    args: &[&str],
    label: &str,
    success_codes: &[i32],
) -> Result<String, OperationalError> {
    let output =
        run_known_program(program, args).map_err(|err| command_error(program, label, err))?;
    command_output_to_string(program, label, success_codes, output)
}

fn run_foxguard_with_cargo_bin(
    args: &[&str],
    success_codes: &[i32],
) -> Result<String, OperationalError> {
    let output = Command::new("foxguard")
        .args(args)
        .env("PATH", path_with_cargo_bin()?)
        .output()
        .map_err(|err| command_error("foxguard", "foxguard", err))?;
    command_output_to_string("foxguard", "foxguard", success_codes, output)
}

fn path_with_cargo_bin() -> Result<OsString, OperationalError> {
    let cargo_bin = foxguard_cargo_bin_dir()?;
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![cargo_bin];
    entries.extend(std::env::split_paths(&existing).map(|path| path.into_os_string()));
    std::env::join_paths(entries).map_err(|err| {
        OperationalError::new(format!(
            "foxguard was installed but PATH could not be updated for this process: {err}"
        ))
    })
}

fn command_output_to_string(
    program: &str,
    label: &str,
    success_codes: &[i32],
    output: std::process::Output,
) -> Result<String, OperationalError> {
    if output
        .status
        .code()
        .map(|code| success_codes.contains(&code))
        .unwrap_or(false)
    {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if program == "gh" && looks_like_auth_error(&stderr) {
        return Err(OperationalError::new(format!(
            "GitHub CLI authentication failed. Run gh auth login.{}",
            suffix_stderr(&stderr)
        )));
    }
    Err(OperationalError::new(format!(
        "{label} command failed with status {}.{}",
        output.status,
        suffix_stderr(&stderr)
    )))
}

fn run_known_program(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    match program {
        "git" => Command::new("git").args(args).output(),
        "gh" => Command::new("gh").args(args).output(),
        "foxguard" => Command::new("foxguard").args(args).output(),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsupported security helper `{other}`"),
        )),
    }
}

fn command_error(program: &str, label: &str, err: std::io::Error) -> OperationalError {
    if err.kind() == std::io::ErrorKind::NotFound && program == "gh" {
        OperationalError::new("GitHub CLI (gh) is required. Install it and run gh auth login.")
    } else if err.kind() == std::io::ErrorKind::NotFound && program == "foxguard" {
        OperationalError::new("foxguard is not installed")
    } else {
        OperationalError::new(format!("failed to execute {label}: {err}"))
    }
}

fn looks_like_auth_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("authentication")
        || lower.contains("auth login")
        || lower.contains("not logged")
}

fn suffix_stderr(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" {stderr}")
    }
}

#[derive(Debug)]
struct OperationalError {
    message: String,
}

impl OperationalError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn is_feature_unavailable(&self) -> bool {
        let lower = self.message.to_ascii_lowercase();
        lower.contains("advanced security must be enabled")
            || lower.contains("code scanning is not enabled")
            || lower.contains("secret scanning is disabled")
            || lower.contains("dependabot alerts are disabled")
            || lower.contains("repository vulnerability alerts are disabled")
    }

    fn is_missing_foxguard(&self) -> bool {
        self.message
            .to_ascii_lowercase()
            .contains("foxguard is not installed")
    }
}

impl std::fmt::Display for OperationalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnerRepo {
    owner: String,
    repo: String,
}

impl std::fmt::Display for OwnerRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

fn parse_owner_repo(raw: &str) -> Result<OwnerRepo, OperationalError> {
    let trimmed = raw.trim().trim_end_matches(".git");
    let parts = trimmed.split('/').collect::<Vec<_>>();
    if parts.len() == 2 && parts.iter().all(|part| !part.is_empty()) {
        return Ok(OwnerRepo {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
        });
    }
    Err(OperationalError::new(
        "invalid repository; expected --repo OWNER/REPO",
    ))
}

fn parse_github_remote(remote: &str) -> Result<OwnerRepo, OperationalError> {
    if let Some(rest) = remote.strip_prefix("https://github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = remote.strip_prefix("ssh://git@github.com/") {
        return parse_owner_repo(rest);
    }
    Err(OperationalError::new(
        "origin remote is not a supported GitHub URL; pass --repo OWNER/REPO",
    ))
}

#[derive(Clone, Debug)]
struct QueryOptions {
    states: Vec<String>,
    severities: Vec<String>,
    security_severities: Vec<String>,
}

impl QueryOptions {
    fn from_args(args: &SecurityArgs) -> Result<Self, OperationalError> {
        let states = parse_filter_list(
            &args.state,
            &["open", "dismissed", "fixed", "resolved", "auto_dismissed"],
            "state",
        )?;
        let severities = match args.severity.as_deref() {
            Some(raw) => parse_filter_list(raw, &["error", "warning", "note"], "severity")?,
            None => Vec::new(),
        };
        let security_severities = match args.security_severity.as_deref() {
            Some(raw) => parse_filter_list(
                raw,
                &["critical", "high", "medium", "low"],
                "security severity",
            )?,
            None => Vec::new(),
        };
        Ok(Self {
            states,
            severities,
            security_severities,
        })
    }

    fn states_for(&self, source: AlertSource) -> Vec<String> {
        let allowed = match source {
            AlertSource::CodeScanning => &["open", "dismissed", "fixed"][..],
            AlertSource::Dependabot => &["open", "dismissed", "fixed", "auto_dismissed"][..],
            AlertSource::SecretScanning => &["open", "resolved"][..],
            AlertSource::Foxguard => &["open"][..],
        };
        self.states
            .iter()
            .filter(|state| allowed.contains(&state.as_str()))
            .cloned()
            .collect()
    }

    fn matches(&self, alert: &Alert) -> bool {
        matches_filter(&self.states, Some(&alert.state))
            && matches_filter(&self.severities, alert.severity.as_ref())
            && matches_filter(&self.security_severities, alert.security_severity.as_ref())
    }
}

fn parse_filter_list(
    raw: &str,
    allowed: &[&str],
    label: &str,
) -> Result<Vec<String>, OperationalError> {
    let mut values = Vec::new();
    for value in raw.split(',') {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if !allowed.contains(&normalized.as_str()) {
            return Err(OperationalError::new(format!(
                "invalid {label} `{normalized}`; expected one of {}",
                allowed.join(", ")
            )));
        }
        if !values.contains(&normalized) {
            values.push(normalized);
        }
    }
    Ok(values)
}

fn matches_filter(filter: &[String], value: Option<&String>) -> bool {
    filter.is_empty()
        || value
            .map(|value| filter.iter().any(|item| item.eq_ignore_ascii_case(value)))
            .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AlertSource {
    CodeScanning,
    Dependabot,
    SecretScanning,
    Foxguard,
}

impl std::fmt::Display for AlertSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSource::CodeScanning => f.write_str("code_scanning"),
            AlertSource::Dependabot => f.write_str("dependabot"),
            AlertSource::SecretScanning => f.write_str("secret_scanning"),
            AlertSource::Foxguard => f.write_str("foxguard"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CodeScanningAlert {
    number: Option<u64>,
    state: Option<String>,
    html_url: Option<String>,
    rule: Option<CodeScanningRule>,
    tool: Option<ApiTool>,
    most_recent_instance: Option<CodeScanningInstance>,
}

#[derive(Clone, Debug, Deserialize)]
struct CodeScanningRule {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    severity: Option<String>,
    security_severity_level: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiTool {
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct CodeScanningInstance {
    message: Option<ApiMessage>,
    location: Option<ApiLocation>,
    tool: Option<ApiTool>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiMessage {
    text: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiLocation {
    path: Option<String>,
    start_line: Option<u64>,
    start_column: Option<u64>,
    end_line: Option<u64>,
    end_column: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct DependabotAlert {
    number: Option<u64>,
    state: Option<String>,
    html_url: Option<String>,
    dependency: Option<DependabotDependency>,
    security_advisory: Option<SecurityAdvisory>,
    security_vulnerability: Option<SecurityVulnerability>,
}

#[derive(Clone, Debug, Deserialize)]
struct DependabotDependency {
    package: Option<Package>,
    manifest_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Package {
    ecosystem: Option<String>,
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SecurityAdvisory {
    ghsa_id: Option<String>,
    cve_id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    severity: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SecurityVulnerability {
    package: Option<Package>,
    vulnerable_version_range: Option<String>,
    first_patched_version: Option<PatchedVersion>,
    severity: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct PatchedVersion {
    identifier: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SecretScanningAlert {
    number: Option<u64>,
    state: Option<String>,
    html_url: Option<String>,
    secret_type: Option<String>,
    secret_type_display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct FoxguardFinding {
    rule_id: Option<String>,
    severity: Option<String>,
    cwe: Option<String>,
    description: Option<String>,
    file: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
    end_line: Option<u64>,
    end_column: Option<u64>,
    snippet: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Alert {
    number: Option<u64>,
    source: AlertSource,
    state: String,
    tool: Option<String>,
    severity: Option<String>,
    security_severity: Option<String>,
    rule_id: Option<String>,
    rule_name: Option<String>,
    rule_description: Option<String>,
    message: Option<String>,
    package: Option<String>,
    ecosystem: Option<String>,
    vulnerable_version_range: Option<String>,
    first_patched_version: Option<String>,
    secret_type: Option<String>,
    path: Option<String>,
    start_line: Option<u64>,
    start_column: Option<u64>,
    end_line: Option<u64>,
    end_column: Option<u64>,
    html_url: Option<String>,
}

fn normalize_code_scanning_alert(alert: CodeScanningAlert) -> Alert {
    let tool = alert
        .tool
        .as_ref()
        .and_then(|tool| tool.name.clone())
        .or_else(|| {
            alert
                .most_recent_instance
                .as_ref()
                .and_then(|instance| instance.tool.as_ref())
                .and_then(|tool| tool.name.clone())
        });
    let rule = alert.rule;
    let instance = alert.most_recent_instance;
    let location = instance
        .as_ref()
        .and_then(|instance| instance.location.as_ref());
    Alert {
        number: alert.number,
        source: AlertSource::CodeScanning,
        state: lower_string(alert.state).unwrap_or_else(|| "unknown".to_string()),
        tool,
        severity: rule
            .as_ref()
            .and_then(|rule| lower_string(rule.severity.clone())),
        security_severity: rule
            .as_ref()
            .and_then(|rule| lower_string(rule.security_severity_level.clone())),
        rule_id: rule.as_ref().and_then(|rule| rule.id.clone()),
        rule_name: rule.as_ref().and_then(|rule| rule.name.clone()),
        rule_description: rule.as_ref().and_then(|rule| rule.description.clone()),
        message: instance
            .as_ref()
            .and_then(|instance| instance.message.as_ref())
            .and_then(|message| message.text.clone()),
        package: None,
        ecosystem: None,
        vulnerable_version_range: None,
        first_patched_version: None,
        secret_type: None,
        path: location.and_then(|location| location.path.clone()),
        start_line: location.and_then(|location| location.start_line),
        start_column: location.and_then(|location| location.start_column),
        end_line: location.and_then(|location| location.end_line),
        end_column: location.and_then(|location| location.end_column),
        html_url: alert.html_url,
    }
}

fn normalize_dependabot_alert(alert: DependabotAlert) -> Alert {
    let advisory = alert.security_advisory;
    let vulnerability = alert.security_vulnerability;
    let dependency = alert.dependency;
    let package = vulnerability
        .as_ref()
        .and_then(|vuln| vuln.package.as_ref())
        .or_else(|| dependency.as_ref().and_then(|dep| dep.package.as_ref()));
    let package_name = package.and_then(|package| package.name.clone());
    let ecosystem = package.and_then(|package| package.ecosystem.clone());
    let rule_id = advisory
        .as_ref()
        .and_then(|advisory| advisory.ghsa_id.clone())
        .or_else(|| {
            advisory
                .as_ref()
                .and_then(|advisory| advisory.cve_id.clone())
        });
    let severity = advisory
        .as_ref()
        .and_then(|advisory| lower_string(advisory.severity.clone()))
        .or_else(|| {
            vulnerability
                .as_ref()
                .and_then(|vuln| lower_string(vuln.severity.clone()))
        });
    Alert {
        number: alert.number,
        source: AlertSource::Dependabot,
        state: lower_string(alert.state).unwrap_or_else(|| "unknown".to_string()),
        tool: Some("Dependabot".to_string()),
        severity: None,
        security_severity: severity,
        rule_id,
        rule_name: advisory
            .as_ref()
            .and_then(|advisory| advisory.summary.clone()),
        rule_description: advisory
            .as_ref()
            .and_then(|advisory| advisory.description.clone()),
        message: dependabot_message(
            package_name.as_deref(),
            ecosystem.as_deref(),
            &vulnerability,
        ),
        package: package_name,
        ecosystem,
        vulnerable_version_range: vulnerability
            .as_ref()
            .and_then(|vuln| vuln.vulnerable_version_range.clone()),
        first_patched_version: vulnerability
            .as_ref()
            .and_then(|vuln| vuln.first_patched_version.as_ref())
            .and_then(|version| version.identifier.clone()),
        secret_type: None,
        path: dependency.and_then(|dependency| dependency.manifest_path),
        start_line: None,
        start_column: None,
        end_line: None,
        end_column: None,
        html_url: alert.html_url,
    }
}

fn normalize_secret_scanning_alert(alert: SecretScanningAlert) -> Alert {
    Alert {
        number: alert.number,
        source: AlertSource::SecretScanning,
        state: lower_string(alert.state).unwrap_or_else(|| "unknown".to_string()),
        tool: Some("GitHub secret scanning".to_string()),
        severity: None,
        security_severity: Some("critical".to_string()),
        rule_id: alert.secret_type.clone(),
        rule_name: alert.secret_type_display_name.clone(),
        rule_description: None,
        message: alert
            .secret_type_display_name
            .as_ref()
            .map(|name| format!("Secret scanning detected {name}.")),
        package: None,
        ecosystem: None,
        vulnerable_version_range: None,
        first_patched_version: None,
        secret_type: alert.secret_type,
        path: None,
        start_line: None,
        start_column: None,
        end_line: None,
        end_column: None,
        html_url: alert.html_url,
    }
}

fn normalize_foxguard_finding(finding: FoxguardFinding, mode: &str) -> Alert {
    let mut message_parts = Vec::new();
    if let Some(description) = &finding.description {
        message_parts.push(description.clone());
    }
    if let Some(cwe) = &finding.cwe {
        message_parts.push(format!("CWE: {cwe}"));
    }
    if let Some(snippet) = &finding.snippet {
        message_parts.push(format!("Snippet: {snippet}"));
    }
    Alert {
        number: None,
        source: AlertSource::Foxguard,
        state: "open".to_string(),
        tool: Some(format!("foxguard {mode}")),
        severity: None,
        security_severity: finding
            .severity
            .and_then(|severity| lower_string(Some(severity))),
        rule_id: finding.rule_id,
        rule_name: finding.cwe,
        rule_description: finding.description,
        message: if message_parts.is_empty() {
            None
        } else {
            Some(message_parts.join("\n"))
        },
        package: None,
        ecosystem: None,
        vulnerable_version_range: None,
        first_patched_version: None,
        secret_type: None,
        path: finding.file,
        start_line: finding.line,
        start_column: finding.column,
        end_line: finding.end_line,
        end_column: finding.end_column,
        html_url: None,
    }
}

fn dependabot_message(
    package: Option<&str>,
    ecosystem: Option<&str>,
    vulnerability: &Option<SecurityVulnerability>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(package) = package {
        parts.push(format!("Package: {package}"));
    }
    if let Some(ecosystem) = ecosystem {
        parts.push(format!("Ecosystem: {ecosystem}"));
    }
    if let Some(range) = vulnerability
        .as_ref()
        .and_then(|vuln| vuln.vulnerable_version_range.as_deref())
    {
        parts.push(format!("Vulnerable range: {range}"));
    }
    if let Some(patched) = vulnerability
        .as_ref()
        .and_then(|vuln| vuln.first_patched_version.as_ref())
        .and_then(|version| version.identifier.as_deref())
    {
        parts.push(format!("First patched version: {patched}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn lower_string(value: Option<String>) -> Option<String> {
    value.map(|value| value.to_ascii_lowercase())
}

#[derive(Debug, Serialize)]
struct Report {
    repository: String,
    branch: String,
    commit: String,
    state_filter: Vec<String>,
    severity_filter: Vec<String>,
    security_severity_filter: Vec<String>,
    warnings: Vec<String>,
    foxguard_report_path: Option<String>,
    alerts: Vec<Alert>,
    summary: Summary,
    #[serde(skip)]
    policy: PolicySummary,
}

struct ReportMetadata {
    repository: String,
    branch: String,
    commit: String,
    state_filter: Vec<String>,
    severity_filter: Vec<String>,
    security_severity_filter: Vec<String>,
    warnings: Vec<String>,
    foxguard_report_path: Option<String>,
}

impl Report {
    fn new(metadata: ReportMetadata, alerts: Vec<Alert>) -> Self {
        let summary = Summary::from_alerts(&alerts);
        let policy = PolicySummary::from_alerts(&alerts);
        Self {
            repository: metadata.repository,
            branch: metadata.branch,
            commit: metadata.commit,
            state_filter: metadata.state_filter,
            severity_filter: metadata.severity_filter,
            security_severity_filter: metadata.security_severity_filter,
            warnings: metadata.warnings,
            foxguard_report_path: metadata.foxguard_report_path,
            alerts,
            summary,
            policy,
        }
    }
}

#[derive(Debug, Serialize)]
struct Summary {
    count: usize,
    by_source: BTreeMap<String, usize>,
    by_severity: BTreeMap<String, usize>,
    by_security_severity: BTreeMap<String, usize>,
}

impl Summary {
    fn from_alerts(alerts: &[Alert]) -> Self {
        let mut by_source = BTreeMap::new();
        let mut by_severity = BTreeMap::new();
        let mut by_security_severity = BTreeMap::new();
        for alert in alerts {
            *by_source.entry(alert.source.to_string()).or_insert(0) += 1;
            if let Some(severity) = &alert.severity {
                *by_severity.entry(severity.clone()).or_insert(0) += 1;
            }
            if let Some(severity) = &alert.security_severity {
                *by_security_severity.entry(severity.clone()).or_insert(0) += 1;
            }
        }
        Self {
            count: alerts.len(),
            by_source,
            by_severity,
            by_security_severity,
        }
    }
}

#[derive(Debug, Serialize)]
struct PolicySummary {
    has_blocking_issues: bool,
    github_issue_count: usize,
    critical_count: usize,
    critical_foxguard_count: usize,
    high_foxguard_needs_investigation_count: usize,
}

impl PolicySummary {
    fn from_alerts(alerts: &[Alert]) -> Self {
        let github_issue_count = alerts
            .iter()
            .filter(|alert| alert.source != AlertSource::Foxguard)
            .count();
        let critical_count = alerts
            .iter()
            .filter(|alert| security_severity_is(alert, "critical"))
            .count();
        let critical_foxguard_count = alerts
            .iter()
            .filter(|alert| {
                alert.source == AlertSource::Foxguard && security_severity_is(alert, "critical")
            })
            .count();
        let high_foxguard_needs_investigation_count = alerts
            .iter()
            .filter(|alert| {
                alert.source == AlertSource::Foxguard
                    && security_severity_is(alert, "high")
                    && categorize_foxguard_alert(alert)
                        == FoxguardTriageCategory::NeedsInvestigation
            })
            .count();
        Self {
            has_blocking_issues: github_issue_count > 0
                || critical_foxguard_count > 0
                || high_foxguard_needs_investigation_count > 0,
            github_issue_count,
            critical_count,
            critical_foxguard_count,
            high_foxguard_needs_investigation_count,
        }
    }
}

fn security_severity_is(alert: &Alert, severity: &str) -> bool {
    alert
        .security_severity
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case(severity))
        .unwrap_or(false)
}

fn render_policy_notice(policy: &PolicySummary, ignore_errors: bool) -> String {
    let status = if policy.has_blocking_issues {
        if ignore_errors { "ignored" } else { "fail" }
    } else {
        "pass"
    };
    format!(
        "Security policy: {status} (GitHub-hosted findings: {}, critical findings: {}, critical FoxGuard findings: {}, high FoxGuard findings needing investigation: {})",
        policy.github_issue_count,
        policy.critical_count,
        policy.critical_foxguard_count,
        policy.high_foxguard_needs_investigation_count
    )
}

fn render_markdown(report: &Report) -> String {
    render_markdown_with_instructions(
        report,
        "Triage and address the following GitHub security and quality findings. Do not change runtime behavior without explicit user agreement. Routine non-behavioral refactors, tests, formatting, FoxGuard config updates, and documented suppressions for proven false positives are pre-authorised. Avoid destructive changes.",
    )
}

fn render_prompt(report: &Report) -> String {
    render_markdown_with_instructions(
        report,
        "You are a coding agent working in this repository. Triage and address the following GitHub security and quality findings end-to-end. Do not change runtime behavior without explicit user agreement. Make routine non-behavioral refactors, add or update tests, run focused verification, create or update FoxGuard config, and add documented suppressions for proven false positives. Stop for permission before destructive, credential-sensitive, or behavior-changing actions.",
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoxguardTriageCategory {
    NeedsHumanConsent,
    LikelyFalsePositive,
    SafeCleanup,
    NeedsInvestigation,
}

impl FoxguardTriageCategory {
    const ALL: [Self; 4] = [
        Self::NeedsHumanConsent,
        Self::LikelyFalsePositive,
        Self::SafeCleanup,
        Self::NeedsInvestigation,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::NeedsHumanConsent => "Needs Human Consent",
            Self::LikelyFalsePositive => "Likely False Positive",
            Self::SafeCleanup => "Safe Cleanup",
            Self::NeedsInvestigation => "Needs Investigation",
        }
    }

    fn strategy(self) -> &'static str {
        match self {
            Self::NeedsHumanConsent => {
                "Strategy: inspect the data flow and propose the smallest safe fix, but stop before changing runtime behavior, accepted inputs, network destinations, command execution, credential handling, or path access without explicit human consent."
            }
            Self::LikelyFalsePositive => {
                "Strategy: verify the flagged expression is not attacker-controlled. If it is a false positive, add a short accepted-risk comment and an exact inline FoxGuard directive such as `// foxguard: ignore[rule-id]` immediately before the flagged line."
            }
            Self::SafeCleanup => {
                "Strategy: prefer a small non-behavioral code cleanup, such as propagating an existing error path, narrowing a test-only/non-distributed exclusion, or documenting an invariant. If the cleanup would alter observable behavior, move it to `Needs Human Consent`."
            }
            Self::NeedsInvestigation => {
                "Strategy: inspect the surrounding code before editing. Classify it into one of the other categories if possible; otherwise document the uncertainty and ask before making behavior-changing or credential-sensitive changes."
            }
        }
    }
}

fn categorize_foxguard_alert(alert: &Alert) -> FoxguardTriageCategory {
    let text = foxguard_alert_text(alert);

    if contains_any(&text, &["map.get", "source_versions.get", "responses.get"]) {
        return FoxguardTriageCategory::LikelyFalsePositive;
    }

    if alert
        .tool
        .as_deref()
        .map(|tool| tool.contains("secrets"))
        .unwrap_or(false)
        || contains_any(&text, &["secret", "token", "credential", "private key"])
    {
        return FoxguardTriageCategory::NeedsHumanConsent;
    }

    match alert.rule_id.as_deref() {
        Some("rs/no-ssrf" | "rs/no-command-injection" | "rs/no-path-traversal") => {
            FoxguardTriageCategory::NeedsHumanConsent
        }
        Some("rs/no-unwrap-in-lib") => FoxguardTriageCategory::SafeCleanup,
        Some("rs/unsafe-block") => FoxguardTriageCategory::NeedsInvestigation,
        _ => FoxguardTriageCategory::NeedsInvestigation,
    }
}

fn foxguard_alert_text(alert: &Alert) -> String {
    [
        alert.rule_id.as_deref(),
        alert.rule_name.as_deref(),
        alert.rule_description.as_deref(),
        alert.message.as_deref(),
        alert.path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
    .to_ascii_lowercase()
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn render_foxguard_report(alerts: &[Alert]) -> String {
    let mut out = String::new();
    let summary = Summary::from_alerts(alerts);
    out.push_str("# FoxGuard Security And Quality Findings\n\n");
    out.push_str(&format!(
        "Found {} FoxGuard finding{}.\n\n",
        summary.count,
        if summary.count == 1 { "" } else { "s" }
    ));
    out.push_str("## Instructions For Codex\n\n");
    out.push_str("- Triage every finding before editing code.\n");
    out.push_str("- Make the smallest possible safe change.\n");
    out.push_str("- Do not change runtime behavior without explicit human consent.\n");
    out.push_str("- If a finding is a false positive, add a short normal comment explaining the accepted risk and then an exact FoxGuard directive on the line immediately before the flagged code, for example `// foxguard: ignore[rule-id]`.\n");
    out.push_str("- Do not add trailing text to a FoxGuard directive; FoxGuard v0.7.1 requires exact directive text.\n");
    out.push_str("- Use `.foxguard.yml` or FoxGuard excludes only for test-only or non-distributed paths. Do not hide production `src/` findings in config unless the file is demonstrably not packaged or executed in production.\n");
    out.push_str("- Do not suppress secret findings with inline comments; remove references and tell the user which credential must be revoked.\n\n");

    out.push_str("## Triage Categories\n\n");
    for category in FoxguardTriageCategory::ALL {
        let count = alerts
            .iter()
            .filter(|alert| categorize_foxguard_alert(alert) == category)
            .count();
        out.push_str(&format!(
            "- {}: {} finding{}. {}\n",
            category.title(),
            count,
            if count == 1 { "" } else { "s" },
            category.strategy()
        ));
    }
    out.push('\n');

    for category in FoxguardTriageCategory::ALL {
        let category_alerts = alerts
            .iter()
            .filter(|alert| categorize_foxguard_alert(alert) == category)
            .collect::<Vec<_>>();
        out.push_str(&format!("## {}\n\n", category.title()));
        out.push_str(category.strategy());
        out.push_str("\n\n");

        if category_alerts.is_empty() {
            out.push_str("No findings in this category.\n\n");
            continue;
        }

        for (idx, alert) in category_alerts.into_iter().enumerate() {
            render_foxguard_alert(&mut out, idx + 1, alert);
        }
    }

    out
}

fn render_foxguard_alert(out: &mut String, idx: usize, alert: &Alert) {
    out.push_str(&format!(
        "### {}. {}\n\n",
        idx,
        display_opt(alert.rule_id.as_deref())
    ));
    out.push_str(&format!("Source: {}\n", alert.source));
    if let Some(tool) = &alert.tool {
        out.push_str(&format!("Tool: {tool}\n"));
    }
    out.push_str(&format!("State: {}\n", alert.state));
    if alert.path.is_some() || alert.start_line.is_some() {
        out.push_str(&format!("File: {}\n", display_location(alert)));
    }
    if let Some(security_severity) = &alert.security_severity {
        out.push_str(&format!("Security severity: {security_severity}\n"));
    }
    if let Some(name) = &alert.rule_name {
        out.push_str(&format!("Name: {name}\n"));
    }
    if let Some(description) = &alert.rule_description {
        out.push_str(&format!("Description: {description}\n"));
    }
    if let Some(message) = &alert.message {
        out.push_str("\nMessage:\n");
        out.push_str(message);
        out.push('\n');
    }
    out.push_str("\nHandling:\n");
    out.push_str("Classify this finding as real, false positive, test-only, non-distributed, or behavior-changing. Fix real findings with the smallest safe non-behavioral change. If behavior would change, stop and ask for explicit human consent. For proven false positives, use an accepted-risk comment plus an exact `foxguard: ignore[rule-id]` directive.\n\n");
}

fn render_markdown_with_instructions(report: &Report, instructions: &str) -> String {
    let mut out = String::new();
    out.push_str("# GitHub Security And Quality Issues\n\n");
    out.push_str(&format!("Repository: {}\n", report.repository));
    out.push_str(&format!("Branch: {}\n", report.branch));
    out.push_str(&format!("Commit: {}\n", report.commit));
    out.push_str(&format!(
        "State filter: {}\n",
        display_filter(&report.state_filter)
    ));
    out.push_str(&format!(
        "Severity filter: {}\n",
        display_filter(&report.severity_filter)
    ));
    out.push_str(&format!(
        "Security severity filter: {}\n\n",
        display_filter(&report.security_severity_filter)
    ));
    if !report.warnings.is_empty() {
        out.push_str("Warnings:\n");
        for warning in &report.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "Found {} security alert{}.\n\n",
        report.summary.count,
        if report.summary.count == 1 { "" } else { "s" }
    ));
    if report.summary.count > 0 {
        out.push_str("## Copy/Paste Instructions For Codex\n\n");
        out.push_str("Triage every finding listed below. Treat GitHub code scanning, Dependabot, secret scanning, and FoxGuard findings as actionable unless the code proves a finding is a false positive. Make the smallest safe non-behavioral changes, run focused verification, and summarize any residual risk. Do not change runtime behavior without explicit user agreement. Do not modify secrets or credentials directly; for secret findings, remove references and tell the user which credential must be revoked.\n\n");
        out.push_str("FoxGuard workflow:\n");
        out.push_str("- `greentic-dev security` creates or updates `.foxguard.yml` before running FoxGuard and adds default `scan.ignore_rules` for test-only or non-distributed paths such as `tests/`, `benches/`, fixtures, examples, generated artifacts, and docs.\n");
        out.push_str("- If additional test-only or non-distributed paths are discovered, add them to `.foxguard.yml` `scan.ignore_rules` when those findings do not affect packaged runtime behavior.\n");
        out.push_str("- Do not hide production `src/` findings in `.foxguard.yml` unless the file is demonstrably not packaged or not executed in production.\n");
        out.push_str("- For individual false positives in production code, add a short normal comment explaining the accepted risk and an exact inline directive on its own line immediately before the flagged code, for example `// foxguard: ignore[rule-id]`.\n");
        out.push_str("- Do not add trailing text to a FoxGuard inline directive; FoxGuard v0.7.1 requires the directive text to be exact.\n");
        out.push_str("- Do not suppress `foxguard secrets` findings with inline comments; remove references and tell the user which credential must be revoked.\n\n");
        if let Some(path) = &report.foxguard_report_path {
            out.push_str("FoxGuard findings are written to a separate report:\n\n");
            out.push_str("Before using the linked report, triage each finding, prefer the smallest possible safe change, do not change runtime behavior without explicit human consent, and resolve proven false positives with exact inline FoxGuard comments.\n\n");
            out.push_str(&format!("- `{path}`\n\n"));
            out.push_str("Open that file for the full FoxGuard output.\n\n");
        }
        if let Some(config) = suggested_foxguard_config(report) {
            out.push_str("Suggested starter `.foxguard.yml` for non-distributed findings detected in this report:\n\n");
            out.push_str("```yaml\n");
            out.push_str(&config);
            out.push_str("```\n\n");
            out.push_str("Review this before applying it. It is intended for test-only or non-distributed code paths, not production `src/` findings.\n\n");
        }
    }
    out.push_str("## Coding-agent instructions\n\n");
    out.push_str(instructions);
    out.push('\n');

    let inline_alerts = report
        .alerts
        .iter()
        .filter(|alert| {
            report.foxguard_report_path.is_none() || alert.source != AlertSource::Foxguard
        })
        .collect::<Vec<_>>();
    if inline_alerts.is_empty() && report.foxguard_report_path.is_some() {
        out.push_str("\nNo GitHub-hosted findings are available in this environment. See the FoxGuard report linked above for local findings.\n");
    }
    for (idx, alert) in inline_alerts.into_iter().enumerate() {
        out.push_str(&format!(
            "\n### {}. {} - {}\n\n",
            idx + 1,
            alert.source,
            display_opt(alert.rule_id.as_deref())
        ));
        out.push_str(&format!("Source: {}\n", alert.source));
        if let Some(tool) = &alert.tool {
            out.push_str(&format!("Tool: {tool}\n"));
        }
        out.push_str(&format!("State: {}\n", alert.state));
        if alert.path.is_some() || alert.start_line.is_some() {
            out.push_str(&format!("File: {}\n", display_location(alert)));
        }
        if let Some(package) = &alert.package {
            out.push_str(&format!("Package: {package}\n"));
        }
        if let Some(ecosystem) = &alert.ecosystem {
            out.push_str(&format!("Ecosystem: {ecosystem}\n"));
        }
        if let Some(severity) = &alert.severity {
            out.push_str(&format!("Severity: {severity}\n"));
        }
        if let Some(security_severity) = &alert.security_severity {
            out.push_str(&format!("Security severity: {security_severity}\n"));
        }
        if let Some(name) = &alert.rule_name {
            out.push_str(&format!("Name: {name}\n"));
        }
        if let Some(description) = &alert.rule_description {
            out.push_str(&format!("Description: {description}\n"));
        }
        if let Some(message) = &alert.message {
            out.push_str("\nMessage:\n");
            out.push_str(message);
            out.push('\n');
        }
        out.push_str("\nSuggested fix:\n");
        out.push_str("Inspect the alert and classify it as real, false positive, test-only, non-distributed, or behavior-changing. Apply the smallest safe non-behavioral change that resolves real findings. If a fix would change behavior, stop and ask the user before implementing. For FoxGuard false positives, prefer exact inline suppressions with an adjacent accepted-risk comment; for test-only or non-distributed paths, prefer `.foxguard.yml` `scan.ignore_rules`. For Dependabot, update or replace the vulnerable dependency when compatible. For secret scanning, revoke the secret and remove references from history or configuration.\n");
        if let Some(url) = &alert.html_url {
            out.push_str("\nAlert:\n");
            out.push_str(url);
            out.push('\n');
        }
    }

    out
}

fn suggested_foxguard_config(report: &Report) -> Option<String> {
    let mut ignores = BTreeMap::<String, BTreeSet<String>>::new();
    for alert in &report.alerts {
        if alert.source != AlertSource::Foxguard || alert.tool.as_deref() != Some("foxguard scan") {
            continue;
        }
        let Some(rule_id) = alert.rule_id.as_deref() else {
            continue;
        };
        let Some(path) = alert
            .path
            .as_deref()
            .and_then(non_distributed_foxguard_path)
        else {
            continue;
        };
        ignores
            .entry(path.to_string())
            .or_default()
            .insert(rule_id.to_string());
    }
    if ignores.is_empty() {
        return None;
    }

    let mut out = String::from("scan:\n  ignore_rules:\n");
    for (path, rules) in ignores {
        out.push_str(&format!("    - path: {path}\n"));
        out.push_str("      rules:\n");
        for rule in rules {
            out.push_str(&format!("        - {rule}\n"));
        }
    }
    Some(out)
}

fn non_distributed_foxguard_path(path: &str) -> Option<&'static str> {
    let path = path.strip_prefix("./").unwrap_or(path);
    [
        "tests/",
        "benches/",
        "docs/",
        "examples/",
        "fixtures/",
        "testdata/",
        "generated/",
    ]
    .into_iter()
    .find(|prefix| path.starts_with(prefix))
}

fn display_filter(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(",")
    }
}

fn display_opt(value: Option<&str>) -> &str {
    value.unwrap_or("(unknown)")
}

fn display_location(alert: &Alert) -> String {
    let Some(path) = &alert.path else {
        return "(unknown)".to_string();
    };
    match alert.start_line {
        Some(line) => format!("{path}:{line}"),
        None => path.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> SecurityArgs {
        SecurityArgs {
            format: SecurityFormat::Markdown,
            prompt: false,
            no_errors: false,
            severity: None,
            security_severity: None,
            state: "open".to_string(),
            repo: Some("greenticai/greentic-dev".to_string()),
            branch: Some("feature/foo".to_string()),
        }
    }

    #[test]
    fn parses_owner_repo() {
        assert_eq!(
            parse_owner_repo("greenticai/greentic-dev").unwrap(),
            OwnerRepo {
                owner: "greenticai".to_string(),
                repo: "greentic-dev".to_string()
            }
        );
    }

    #[test]
    fn parses_github_https_remote() {
        assert_eq!(
            parse_github_remote("https://github.com/greenticai/greentic-dev.git")
                .unwrap()
                .to_string(),
            "greenticai/greentic-dev"
        );
    }

    #[test]
    fn parses_github_ssh_remotes() {
        assert_eq!(
            parse_github_remote("git@github.com:greenticai/greentic-dev.git")
                .unwrap()
                .to_string(),
            "greenticai/greentic-dev"
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/greenticai/greentic-dev.git")
                .unwrap()
                .to_string(),
            "greenticai/greentic-dev"
        );
    }

    #[test]
    fn rejects_non_github_remotes() {
        assert!(parse_github_remote("https://example.com/owner/repo.git").is_err());
    }

    #[test]
    fn parses_comma_separated_filters() {
        assert_eq!(
            parse_filter_list("error, warning,error", &["error", "warning"], "severity").unwrap(),
            vec!["error", "warning"]
        );
    }

    #[test]
    fn ignore_errors_does_not_change_query_filters() {
        let mut args = args();
        args.no_errors = true;
        args.state = "dismissed".to_string();
        let options = QueryOptions::from_args(&args).unwrap();
        assert_eq!(options.states, vec!["dismissed"]);
        assert!(options.severities.is_empty());
    }

    #[test]
    fn source_specific_states_are_selected() {
        let mut args = args();
        args.state = "open,dismissed,resolved,auto_dismissed".to_string();
        let options = QueryOptions::from_args(&args).unwrap();
        assert_eq!(
            options.states_for(AlertSource::CodeScanning),
            vec!["open", "dismissed"]
        );
        assert_eq!(
            options.states_for(AlertSource::Dependabot),
            vec!["open", "dismissed", "auto_dismissed"]
        );
        assert_eq!(
            options.states_for(AlertSource::SecretScanning),
            vec!["open", "resolved"]
        );
    }

    #[test]
    fn parses_code_scanning_alerts_without_tool_filtering() {
        let alerts: Vec<CodeScanningAlert> = serde_json::from_str(code_scanning_alerts()).unwrap();
        let normalized = alerts
            .into_iter()
            .map(normalize_code_scanning_alert)
            .collect::<Vec<_>>();
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].source, AlertSource::CodeScanning);
        assert_eq!(normalized[0].tool.as_deref(), Some("CodeQL"));
        assert_eq!(normalized[1].tool.as_deref(), Some("OtherScanner"));
    }

    #[test]
    fn parses_dependabot_alerts() {
        let alerts: Vec<DependabotAlert> = serde_json::from_str(dependabot_alerts()).unwrap();
        let normalized = alerts
            .into_iter()
            .map(normalize_dependabot_alert)
            .collect::<Vec<_>>();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].source, AlertSource::Dependabot);
        assert_eq!(normalized[0].security_severity.as_deref(), Some("high"));
        assert_eq!(normalized[0].package.as_deref(), Some("openssl"));
    }

    #[test]
    fn parses_secret_scanning_alerts() {
        let alerts: Vec<SecretScanningAlert> =
            serde_json::from_str(secret_scanning_alerts()).unwrap();
        let normalized = alerts
            .into_iter()
            .map(normalize_secret_scanning_alert)
            .collect::<Vec<_>>();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].source, AlertSource::SecretScanning);
        assert_eq!(normalized[0].security_severity.as_deref(), Some("critical"));
    }

    #[test]
    fn parses_foxguard_findings() {
        let normalized = parse_foxguard_findings(foxguard_findings(), "scan").unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].source, AlertSource::Foxguard);
        assert_eq!(normalized[0].tool.as_deref(), Some("foxguard scan"));
        assert_eq!(normalized[0].security_severity.as_deref(), Some("medium"));
        assert_eq!(normalized[0].path.as_deref(), Some("./src/main.rs"));
    }

    #[test]
    fn creates_default_foxguard_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(".foxguard.yml");
        let message = ensure_default_foxguard_config_at(&path)
            .unwrap()
            .expect("config should be created");
        assert!(message.contains("created"));
        let config = fs::read_to_string(path).unwrap();
        assert!(config.contains("path: tests/"));
        assert!(config.contains("path: benches/"));
        assert!(config.contains("rs/no-unwrap-in-lib"));
        assert!(config.contains("rs/no-path-traversal"));
        assert!(!config.contains("path: src/"));
    }

    #[test]
    fn updates_existing_foxguard_config_without_dropping_existing_rules() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(".foxguard.yml");
        fs::write(
            &path,
            "secrets:\n  baseline: .foxguard/secrets-baseline.json\nscan:\n  ignore_rules:\n    - path: tests/\n      rules:\n        - custom/rule\n",
        )
        .unwrap();
        let message = ensure_default_foxguard_config_at(&path)
            .unwrap()
            .expect("config should be updated");
        assert!(message.contains("updated"));
        let config = fs::read_to_string(path).unwrap();
        assert!(config.contains("baseline: .foxguard/secrets-baseline.json"));
        assert!(config.contains("path: tests/"));
        assert!(config.contains("custom/rule"));
        assert!(config.contains("rs/no-unwrap-in-lib"));
        assert!(config.contains("path: benches/"));
    }

    #[test]
    fn filters_by_severity_and_security_severity() {
        let options = QueryOptions {
            states: vec!["open".to_string()],
            severities: vec!["error".to_string()],
            security_severities: vec!["high".to_string()],
        };
        let alert = normalize_code_scanning_alert(
            serde_json::from_str::<Vec<CodeScanningAlert>>(code_scanning_alerts()).unwrap()[0]
                .clone(),
        );
        assert!(options.matches(&alert));
        let dep = normalize_dependabot_alert(
            serde_json::from_str::<Vec<DependabotAlert>>(dependabot_alerts()).unwrap()[0].clone(),
        );
        assert!(!options.matches(&dep));
    }

    #[test]
    fn renders_empty_alert_list() {
        let report = Report::new(
            report_metadata(
                "greenticai/greentic-dev",
                "main",
                "abc123",
                vec!["open".to_string()],
                None,
            ),
            Vec::new(),
        );
        let markdown = render_markdown(&report);
        assert!(markdown.contains("Found 0 security alerts."));
    }

    #[test]
    fn renders_markdown_report() {
        let report = sample_report();
        let markdown = render_markdown(&report);
        assert!(markdown.contains("# GitHub Security And Quality Issues"));
        assert!(markdown.contains("Copy/Paste Instructions For Codex"));
        assert!(markdown.contains("Source: dependabot"));
        assert!(markdown.contains(FOXGUARD_REPORT_PATH));
        assert!(markdown.contains("FoxGuard findings are written to a separate report"));
        assert!(!markdown.contains("## Policy Flags"));
        assert!(markdown.contains("Package: openssl"));
        assert!(
            markdown.contains("Do not change runtime behavior without explicit user agreement")
        );
        assert!(markdown.contains("creates or updates `.foxguard.yml` before running FoxGuard"));
        assert!(markdown.contains("scan.ignore_rules"));
        assert!(markdown.contains("FoxGuard v0.7.1 requires the directive text to be exact"));
    }

    #[test]
    fn renders_prompt_report() {
        let prompt = render_prompt(&sample_report());
        assert!(prompt.contains("You are a coding agent"));
        assert!(prompt.contains("Triage and address"));
        assert!(prompt.contains("Stop for permission"));
    }

    #[test]
    fn renders_foxguard_report_with_safety_instructions() {
        let alerts = parse_foxguard_findings(foxguard_findings(), "scan").unwrap();
        let report = render_foxguard_report(&alerts);
        assert!(report.contains("# FoxGuard Security And Quality Findings"));
        assert!(!report.contains("## Policy Flags"));
        assert!(report.contains("Do not change runtime behavior without explicit human consent"));
        assert!(report.contains("Make the smallest possible safe change"));
        assert!(report.contains("foxguard: ignore[rule-id]"));
        assert!(report.contains("## Triage Categories"));
        assert!(report.contains("## Safe Cleanup"));
        assert!(report.contains("Source: foxguard"));
    }

    #[test]
    fn renders_foxguard_report_grouped_by_triage_category() {
        let alerts = parse_foxguard_findings(foxguard_category_findings(), "scan").unwrap();
        let report = render_foxguard_report(&alerts);
        assert!(report.contains("Needs Human Consent: 1 finding"));
        assert!(report.contains("Likely False Positive: 1 finding"));
        assert!(report.contains("Safe Cleanup: 1 finding"));
        assert!(report.contains("Needs Investigation: 1 finding"));
        assert!(report.contains("stop before changing runtime behavior"));
        assert!(report.contains("add a short accepted-risk comment"));
        assert!(report.contains("propagating an existing error path"));
        assert!(report.contains("Classify it into one of the other categories"));
    }

    #[test]
    fn renders_json_report() {
        let json = serde_json::to_value(sample_report()).unwrap();
        assert_eq!(json["summary"]["count"], 5);
        assert_eq!(json["summary"]["by_source"]["dependabot"], 1);
        assert_eq!(json["summary"]["by_source"]["foxguard"], 1);
        assert_eq!(json["alerts"][0]["source"], "code_scanning");
        assert!(json.get("policy").is_none());
    }

    #[test]
    fn prompt_overrides_json_format() {
        let mut args = args();
        args.prompt = true;
        args.format = SecurityFormat::Json;
        let runner = MockRunner::default();
        let outcome = run_inner(&runner, args).unwrap();
        assert!(
            outcome
                .output
                .starts_with("# GitHub Security And Quality Issues")
        );
    }

    #[test]
    fn policy_exit_code_2_when_blocking_issues_exist() {
        let runner = MockRunner::default();
        let outcome = run_inner(&runner, args()).unwrap();
        assert_eq!(outcome.exit_code, EXIT_POLICY_VIOLATION);
        assert!(outcome.policy_notice.starts_with("Security policy: fail"));
        assert!(outcome.policy_notice.contains("GitHub-hosted findings: 4"));
    }

    #[test]
    fn ignore_errors_returns_success_when_blocking_issues_exist() {
        let mut args = args();
        args.no_errors = true;
        let runner = MockRunner::default();
        let outcome = run_inner(&runner, args).unwrap();
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
        assert!(
            outcome
                .policy_notice
                .starts_with("Security policy: ignored")
        );
    }

    #[test]
    fn success_exit_code_0_when_no_policy_blocking_alerts_exist() {
        let mut args = args();
        args.security_severity = Some("low".to_string());
        let runner = MockRunner::default();
        let outcome = run_inner(&runner, args).unwrap();
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
        assert!(outcome.policy_notice.starts_with("Security policy: pass"));
    }

    #[test]
    fn policy_blocks_high_foxguard_findings_needing_investigation() {
        let alerts =
            parse_foxguard_findings(foxguard_high_investigation_findings(), "scan").unwrap();
        let report = Report::new(
            report_metadata(
                "greenticai/greentic-dev",
                "feature/foo",
                "abc123",
                vec!["open".to_string()],
                Some(FOXGUARD_REPORT_PATH.to_string()),
            ),
            alerts,
        );
        assert!(report.policy.has_blocking_issues);
        assert_eq!(report.policy.high_foxguard_needs_investigation_count, 1);
        let notice = render_policy_notice(&report.policy, false);
        assert!(notice.contains("high FoxGuard findings needing investigation: 1"));
    }

    #[test]
    fn operational_error_for_git_failures() {
        let runner = MockRunner {
            git_error: Some("git failed".to_string()),
            ..Default::default()
        };
        assert!(run_inner(&runner, args()).is_err());
    }

    #[test]
    fn unavailable_code_scanning_does_not_abort_other_sources() {
        let runner = MockRunner {
            code_scanning_error: Some("gh: Advanced Security must be enabled for this repository to use code scanning. (HTTP 403)".to_string()),
            ..Default::default()
        };
        let mut args = args();
        args.no_errors = true;
        let outcome = run_inner(&runner, args).unwrap();
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
        assert!(outcome.output.contains("dependabot"));
        assert!(outcome.output.contains("secret_scanning"));
    }

    fn sample_report() -> Report {
        let mut alerts = Vec::new();
        alerts.extend(
            serde_json::from_str::<Vec<CodeScanningAlert>>(code_scanning_alerts())
                .unwrap()
                .into_iter()
                .map(normalize_code_scanning_alert),
        );
        alerts.extend(
            serde_json::from_str::<Vec<DependabotAlert>>(dependabot_alerts())
                .unwrap()
                .into_iter()
                .map(normalize_dependabot_alert),
        );
        alerts.extend(
            serde_json::from_str::<Vec<SecretScanningAlert>>(secret_scanning_alerts())
                .unwrap()
                .into_iter()
                .map(normalize_secret_scanning_alert),
        );
        alerts.extend(parse_foxguard_findings(foxguard_findings(), "scan").unwrap());
        Report::new(
            report_metadata(
                "greenticai/greentic-dev",
                "feature/foo",
                "abc123",
                vec!["open".to_string()],
                Some(FOXGUARD_REPORT_PATH.to_string()),
            ),
            alerts,
        )
    }

    fn report_metadata(
        repository: &str,
        branch: &str,
        commit: &str,
        state_filter: Vec<String>,
        foxguard_report_path: Option<String>,
    ) -> ReportMetadata {
        ReportMetadata {
            repository: repository.to_string(),
            branch: branch.to_string(),
            commit: commit.to_string(),
            state_filter,
            severity_filter: Vec::new(),
            security_severity_filter: Vec::new(),
            warnings: Vec::new(),
            foxguard_report_path,
        }
    }

    fn code_scanning_alerts() -> &'static str {
        r#"[
          {
            "number": 1,
            "state": "open",
            "html_url": "https://github.com/OWNER/REPO/security/code-scanning/1",
            "tool": {"name": "CodeQL"},
            "rule": {
              "id": "rust/path-injection",
              "name": "Path injection",
              "description": "User-controlled data flows into a filesystem path.",
              "severity": "error",
              "security_severity_level": "high"
            },
            "most_recent_instance": {
              "message": {"text": "User-controlled data flows into a filesystem path."},
              "location": {
                "path": "crates/foo/src/bar.rs",
                "start_line": 44,
                "start_column": 12,
                "end_line": 44,
                "end_column": 31
              }
            }
          },
          {
            "number": 2,
            "state": "open",
            "html_url": "https://github.com/OWNER/REPO/security/code-scanning/2",
            "tool": {"name": "OtherScanner"},
            "rule": {
              "id": "custom/style",
              "name": "Style issue",
              "severity": "warning"
            }
          }
        ]"#
    }

    fn dependabot_alerts() -> &'static str {
        r#"[
          {
            "number": 3,
            "state": "open",
            "html_url": "https://github.com/OWNER/REPO/security/dependabot/3",
            "dependency": {
              "package": {"ecosystem": "cargo", "name": "openssl"},
              "manifest_path": "Cargo.lock"
            },
            "security_advisory": {
              "ghsa_id": "GHSA-xxxx-yyyy-zzzz",
              "cve_id": "CVE-2026-0001",
              "summary": "openssl vulnerability",
              "description": "A vulnerable dependency is present.",
              "severity": "high"
            },
            "security_vulnerability": {
              "package": {"ecosystem": "cargo", "name": "openssl"},
              "vulnerable_version_range": "< 1.0.0",
              "first_patched_version": {"identifier": "1.0.0"},
              "severity": "high"
            }
          }
        ]"#
    }

    fn secret_scanning_alerts() -> &'static str {
        r#"[
          {
            "number": 4,
            "state": "open",
            "html_url": "https://github.com/OWNER/REPO/security/secret-scanning/4",
            "secret_type": "github_personal_access_token",
            "secret_type_display_name": "GitHub personal access token",
            "secret": "ghp_example"
          }
        ]"#
    }

    fn foxguard_findings() -> &'static str {
        r#"[
          {
            "rule_id": "rs/no-unwrap-in-lib",
            "severity": "medium",
            "cwe": "CWE-248",
            "description": ".unwrap() can panic at runtime",
            "file": "./src/main.rs",
            "line": 10,
            "column": 20,
            "end_line": 10,
            "end_column": 28,
            "snippet": "value.unwrap()"
          }
        ]"#
    }

    fn foxguard_category_findings() -> &'static str {
        r#"[
          {
            "rule_id": "rs/no-ssrf",
            "severity": "high",
            "description": "User-controlled URL passed to HTTP client",
            "file": "./src/install.rs",
            "line": 10,
            "snippet": "client.get(url)"
          },
          {
            "rule_id": "rs/no-ssrf",
            "severity": "high",
            "description": "Possible SSRF",
            "file": "./src/distributor.rs",
            "line": 20,
            "snippet": "map.get(key)"
          },
          {
            "rule_id": "rs/no-unwrap-in-lib",
            "severity": "medium",
            "description": ".unwrap() can panic at runtime",
            "file": "./src/main.rs",
            "line": 30,
            "snippet": "value.unwrap()"
          },
          {
            "rule_id": "rs/unsafe-block",
            "severity": "medium",
            "description": "unsafe block requires review",
            "file": "./src/platform.rs",
            "line": 40,
            "snippet": "unsafe { call() }"
          }
        ]"#
    }

    fn foxguard_high_investigation_findings() -> &'static str {
        r#"[
          {
            "rule_id": "rs/unsafe-block",
            "severity": "high",
            "description": "unsafe block requires review",
            "file": "./src/platform.rs",
            "line": 40,
            "snippet": "unsafe { call() }"
          }
        ]"#
    }

    #[derive(Default)]
    struct MockRunner {
        git_error: Option<String>,
        code_scanning_error: Option<String>,
    }

    impl Runner for MockRunner {
        fn git(&self, args: &[&str]) -> Result<String, OperationalError> {
            if let Some(err) = &self.git_error {
                return Err(OperationalError::new(err));
            }
            match args {
                ["rev-parse", "HEAD"] => Ok("abc123\n".to_string()),
                _ => Ok(String::new()),
            }
        }

        fn gh_api(&self, endpoint: &str) -> Result<String, OperationalError> {
            if endpoint.contains("code-scanning") {
                if let Some(err) = &self.code_scanning_error {
                    return Err(OperationalError::new(err));
                }
                Ok(code_scanning_alerts().to_string())
            } else if endpoint.contains("dependabot") {
                Ok(dependabot_alerts().to_string())
            } else if endpoint.contains("secret-scanning") {
                Ok(secret_scanning_alerts().to_string())
            } else {
                Err(OperationalError::new(format!(
                    "unexpected endpoint {endpoint}"
                )))
            }
        }

        fn ensure_foxguard_config(&self) -> Result<Option<String>, OperationalError> {
            Ok(None)
        }

        fn write_report_file(
            &self,
            _path: &str,
            _content: &str,
        ) -> Result<String, OperationalError> {
            Ok(FOXGUARD_REPORT_PATH.to_string())
        }

        fn local_command(&self, program: &str, args: &[&str]) -> Result<String, OperationalError> {
            assert_eq!(program, "foxguard");
            if args.first() == Some(&"secrets") {
                Ok("[]".to_string())
            } else {
                Ok(foxguard_findings().to_string())
            }
        }
    }
}
