use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamMode {
    Inherit,
    Capture,
}

pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub current_dir: Option<PathBuf>,
    pub stdout: StreamMode,
    pub stderr: StreamMode,
}

impl CommandSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            current_dir: None,
            stdout: StreamMode::Inherit,
            stderr: StreamMode::Inherit,
        }
    }
}

pub struct CommandOutput {
    pub status: ExitStatus,
    #[allow(dead_code)]
    pub stdout: Option<Vec<u8>>,
    pub stderr: Option<Vec<u8>>,
}

pub fn run(spec: CommandSpec) -> Result<CommandOutput> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(dir) = &spec.current_dir {
        command.current_dir(dir);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }

    match (spec.stdout, spec.stderr) {
        (StreamMode::Inherit, StreamMode::Inherit) => {
            command.stdout(Stdio::inherit());
            command.stderr(Stdio::inherit());
            let status = command
                .status()
                .with_context(|| format!("failed to spawn `{}`", spec.program.to_string_lossy()))?;
            Ok(CommandOutput {
                status,
                stdout: None,
                stderr: None,
            })
        }
        (StreamMode::Capture, StreamMode::Capture) => {
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());
            let output = command
                .output()
                .with_context(|| format!("failed to spawn `{}`", spec.program.to_string_lossy()))?;
            Ok(CommandOutput {
                status: output.status,
                stdout: Some(output.stdout),
                stderr: Some(output.stderr),
            })
        }
        _ => anyhow::bail!("mixed capture/inherit mode is not supported yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandSpec, StreamMode, run};
    use std::ffi::OsString;

    #[test]
    fn capture_mode_collects_stdout_and_stderr() {
        let mut spec = CommandSpec::new("sh");
        spec.args = vec![
            OsString::from("-c"),
            OsString::from("printf hello; printf world >&2"),
        ];
        spec.stdout = StreamMode::Capture;
        spec.stderr = StreamMode::Capture;

        let output = run(spec).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.unwrap(), b"hello");
        assert_eq!(output.stderr.unwrap(), b"world");
    }

    #[test]
    fn inherit_mode_returns_status_without_buffers() {
        let mut spec = CommandSpec::new("sh");
        spec.args = vec![OsString::from("-c"), OsString::from("exit 0")];

        let output = run(spec).unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_none());
        assert!(output.stderr.is_none());
    }

    #[test]
    fn mixed_modes_are_rejected() {
        let mut spec = CommandSpec::new("sh");
        spec.args = vec![OsString::from("-c"), OsString::from("exit 0")];
        spec.stdout = StreamMode::Capture;
        spec.stderr = StreamMode::Inherit;

        let err = run(spec).err().expect("expected mixed-mode failure");
        assert!(err.to_string().contains("mixed capture/inherit mode"));
    }
}
