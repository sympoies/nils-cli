use std::io::{self, BufRead, Write};

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("aborted")]
    Aborted,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn confirm(prompt: &str) -> Result<bool, PromptError> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    confirm_with_io(prompt, &mut input, &mut output)
}

pub fn confirm_with_io(
    prompt: &str,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<bool, PromptError> {
    write!(output, "{prompt}")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;
    let trimmed = line.trim_end_matches(['\n', '\r']);
    Ok(matches!(trimmed, "y" | "Y"))
}

pub fn confirm_or_abort(prompt: &str) -> Result<(), PromptError> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    confirm_or_abort_with_io(prompt, &mut input, &mut output)
}

pub fn confirm_or_abort_with_io(
    prompt: &str,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), PromptError> {
    if confirm_with_io(prompt, input, output)? {
        return Ok(());
    }

    writeln!(output, "🚫 Aborted")?;
    Err(PromptError::Aborted)
}

pub fn select_menu_with_io(
    prompt: &str,
    valid_choices: &[&str],
    default_choice: &str,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<String, PromptError> {
    write!(output, "{prompt}")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;
    let trimmed = line.trim_end_matches(['\n', '\r']).trim();
    let choice = if trimmed.is_empty() {
        default_choice
    } else {
        trimmed
    };

    if valid_choices.contains(&choice) {
        return Ok(choice.to_string());
    }

    writeln!(output, "🚫 Aborted")?;
    Err(PromptError::Aborted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use std::io::Cursor;

    #[test]
    fn confirm_accepts_only_y_or_uppercase_y() {
        let mut out: Vec<u8> = Vec::new();

        let mut input = Cursor::new("y\n");
        assert_eq!(confirm_with_io("?", &mut input, &mut out).unwrap(), true);

        let mut input = Cursor::new("Y\n");
        assert_eq!(confirm_with_io("?", &mut input, &mut out).unwrap(), true);

        let mut input = Cursor::new("yes\n");
        assert_eq!(confirm_with_io("?", &mut input, &mut out).unwrap(), false);

        let mut input = Cursor::new("\n");
        assert_eq!(confirm_with_io("?", &mut input, &mut out).unwrap(), false);
    }

    #[test]
    fn confirm_or_abort_prints_aborted_and_errors_on_decline() {
        let mut out: Vec<u8> = Vec::new();
        let mut input = Cursor::new("n\n");

        let err = confirm_or_abort_with_io("prompt ", &mut input, &mut out).unwrap_err();
        assert_ne!(out.len(), 0);
        assert_eq!(String::from_utf8_lossy(&out), "prompt 🚫 Aborted\n");
        assert!(matches!(err, PromptError::Aborted));
    }

    #[test]
    fn select_menu_defaults_and_aborts_on_invalid() {
        let mut out: Vec<u8> = Vec::new();

        let mut input = Cursor::new("\n");
        let v = select_menu_with_io("choose ", &["1", "2"], "2", &mut input, &mut out).unwrap();
        assert_eq!(v, "2");

        let mut out: Vec<u8> = Vec::new();
        let mut input = Cursor::new("nope\n");
        let err =
            select_menu_with_io("choose ", &["1", "2"], "2", &mut input, &mut out).unwrap_err();
        assert!(matches!(err, PromptError::Aborted));
        assert_eq!(String::from_utf8_lossy(&out), "choose 🚫 Aborted\n");
    }
}
