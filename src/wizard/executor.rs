use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{collections::BTreeMap, process};

use anyhow::{Result, bail};

use crate::wizard::plan::{WizardPlan, WizardStep};

pub struct ExecuteOptions {
    pub unsafe_commands: bool,
    pub allow_destructive: bool,
}

pub struct ExecutionReport {
    pub resolved_versions: BTreeMap<String, String>,
    pub commands_executed: usize,
}

pub fn execute(
    plan: &WizardPlan,
    exec_log_path: &std::path::Path,
    opts: &ExecuteOptions,
) -> Result<ExecutionReport> {
    if !opts.allow_destructive && plan_has_destructive_step(plan) {
        bail!("plan requested destructive operations; re-run with --allow-destructive");
    }

    let mut version_cache = BTreeMap::<String, String>::new();
    let mut executed = 0usize;

    for step in &plan.steps {
        if let WizardStep::RunCommand(cmd) = step {
            if !opts.unsafe_commands && !is_allowed_program(&cmd.program) {
                bail!(
                    "command `{}` is not allowed by default; use --unsafe-commands to allow it",
                    cmd.program
                );
            }
            if args_look_unsafe(&cmd.args) {
                bail!(
                    "command `{}` contains blocked shell-like arguments; refusing to execute",
                    cmd.program
                );
            }

            if let Some(actual_version) = resolve_program_version(&cmd.program)? {
                validate_version_pin(plan, &cmd.program, &actual_version)?;
                version_cache
                    .entry(cmd.program.clone())
                    .or_insert(actual_version.clone());
            }

            append_exec_log(exec_log_path, &cmd.program, &cmd.args)?;

            let status = Command::new(&cmd.program)
                .args(&cmd.args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()?;
            if !status.success() {
                bail!(
                    "wizard step command failed: {} {:?} (exit code {:?})",
                    cmd.program,
                    cmd.args,
                    status.code()
                );
            }
            executed += 1;
        }
    }
    Ok(ExecutionReport {
        resolved_versions: version_cache,
        commands_executed: executed,
    })
}

fn append_exec_log(path: &std::path::Path, program: &str, args: &[String]) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "RUN {} {}", program, args.join(" "))?;
    Ok(())
}

fn validate_version_pin(plan: &WizardPlan, program: &str, actual_version: &str) -> Result<()> {
    let key = format!("resolved_versions.{program}");
    if let Some(expected) = plan.inputs.get(&key)
        && expected != actual_version
    {
        bail!(
            "replay pin mismatch for `{}`: expected `{}`, got `{}`",
            program,
            expected,
            actual_version
        );
    }
    Ok(())
}

fn resolve_program_version(program: &str) -> Result<Option<String>> {
    let output: process::Output = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|s| s.to_string());
    Ok(version_line)
}

fn is_allowed_program(program: &str) -> bool {
    matches!(
        program,
        "greentic-pack"
            | "greentic-component"
            | "greentic-bundle"
            | "greentic-flow"
            | "greentic-operator"
            | "greentic-runner-cli"
    )
}

fn args_look_unsafe(args: &[String]) -> bool {
    const BLOCKED: &[&str] = &["|", ";", "&&", "||", "sh", "-c", "rm", "mv", "dd"];
    args.iter().any(|arg| BLOCKED.contains(&arg.as_str()))
}

fn plan_has_destructive_step(_plan: &WizardPlan) -> bool {
    _plan.steps.iter().any(|step| match step {
        WizardStep::RunCommand(cmd) => cmd.destructive,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{args_look_unsafe, is_allowed_program, validate_version_pin};
    use crate::wizard::plan::{WizardFrontend, WizardPlan, WizardPlanMetadata};

    #[test]
    fn allowlist_basics() {
        assert!(is_allowed_program("greentic-flow"));
        assert!(!is_allowed_program("bash"));
    }

    #[test]
    fn unsafe_args_blocked() {
        assert!(args_look_unsafe(&["-c".to_string()]));
        assert!(args_look_unsafe(&["rm".to_string()]));
        assert!(!args_look_unsafe(&[
            "doctor".to_string(),
            "--json".to_string()
        ]));
    }

    #[test]
    fn version_pin_accepts_match() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "resolved_versions.greentic-flow".to_string(),
            "greentic-flow 0.4.99".to_string(),
        );
        let plan = WizardPlan {
            plan_version: 1,
            created_at: None,
            metadata: WizardPlanMetadata {
                target: "flow".to_string(),
                mode: "create".to_string(),
                locale: "en-US".to_string(),
                frontend: WizardFrontend::Json,
            },
            inputs,
            steps: vec![],
        };
        validate_version_pin(&plan, "greentic-flow", "greentic-flow 0.4.99").expect("match");
    }

    #[test]
    fn version_pin_rejects_mismatch() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "resolved_versions.greentic-flow".to_string(),
            "greentic-flow 0.4.10".to_string(),
        );
        let plan = WizardPlan {
            plan_version: 1,
            created_at: None,
            metadata: WizardPlanMetadata {
                target: "flow".to_string(),
                mode: "create".to_string(),
                locale: "en-US".to_string(),
                frontend: WizardFrontend::Json,
            },
            inputs,
            steps: vec![],
        };
        let err = validate_version_pin(&plan, "greentic-flow", "greentic-flow 0.4.11")
            .expect_err("expected mismatch");
        assert!(err.to_string().contains("replay pin mismatch"));
    }
}
