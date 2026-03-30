use std::io::{self, IsTerminal, Write};

use anyhow::{Result, bail};

use crate::i18n;

pub fn ensure_execute_allowed(
    summary: &str,
    yes: bool,
    non_interactive: bool,
    locale: &str,
) -> Result<()> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    ensure_execute_allowed_with(summary, yes, non_interactive, locale, interactive, || {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input)
    })
}

fn ensure_execute_allowed_with<F>(
    summary: &str,
    yes: bool,
    non_interactive: bool,
    locale: &str,
    interactive: bool,
    read_input: F,
) -> Result<()>
where
    F: FnOnce() -> Result<String>,
{
    if yes {
        return Ok(());
    }

    if !interactive {
        if non_interactive {
            return Ok(());
        }
        bail!(
            "{}",
            i18n::t(locale, "runtime.wizard.confirm.error.non_interactive")
        );
    }

    eprintln!("{summary}");
    eprint!("{}", i18n::t(locale, "runtime.wizard.confirm.prompt"));
    io::stderr().flush()?;
    let input = read_input()?;
    let accepted = matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if accepted {
        Ok(())
    } else {
        bail!(
            "{}",
            i18n::t(locale, "runtime.wizard.confirm.error.canceled")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_execute_allowed_with;

    #[test]
    fn yes_flag_bypasses_confirmation() {
        ensure_execute_allowed_with("summary", true, false, "en", true, || {
            panic!("should not prompt")
        })
        .unwrap();
    }

    #[test]
    fn non_interactive_mode_allows_execution_without_tty() {
        ensure_execute_allowed_with("summary", false, true, "en", false, || {
            panic!("should not prompt")
        })
        .unwrap();
    }

    #[test]
    fn non_interactive_without_opt_in_is_rejected() {
        let err = ensure_execute_allowed_with("summary", false, false, "en", false, || {
            panic!("should not prompt")
        })
        .unwrap_err();
        assert!(err.to_string().contains("non-interactive"));
    }

    #[test]
    fn interactive_decline_is_rejected() {
        let err = ensure_execute_allowed_with("summary", false, false, "en", true, || {
            Ok("n\n".to_string())
        })
        .unwrap_err();
        assert!(err.to_string().contains("canceled"));
    }

    #[test]
    fn interactive_accept_is_allowed() {
        ensure_execute_allowed_with("summary", false, false, "en", true, || {
            Ok("yes\n".to_string())
        })
        .unwrap();
    }
}
