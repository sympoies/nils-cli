use anyhow::Result;
use std::io::{self, BufRead, Write};

pub fn confirm(prompt: &str) -> Result<bool> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    confirm_with_io(prompt, &mut input, &mut output)
}

pub fn confirm_with_io<R: BufRead, W: Write>(
    prompt: &str,
    input: &mut R,
    output: &mut W,
) -> Result<bool> {
    write!(output, "{prompt}")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    let trimmed = line.trim();
    if trimmed == "y" || trimmed == "Y" {
        return Ok(true);
    }

    writeln!(output, "🚫 Aborted")?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::confirm_with_io;
    use pretty_assertions::assert_eq;
    use std::io::Cursor;

    #[test]
    fn confirm_with_io_accepts_lowercase_y() {
        let mut input = Cursor::new("y\n");
        let mut output = Vec::new();
        let ok = confirm_with_io("Prompt ", &mut input, &mut output).expect("confirm");
        assert!(ok);
        assert_eq!(String::from_utf8_lossy(&output), "Prompt ");
    }

    #[test]
    fn confirm_with_io_accepts_uppercase_y() {
        let mut input = Cursor::new("Y\n");
        let mut output = Vec::new();
        let ok = confirm_with_io("Prompt ", &mut input, &mut output).expect("confirm");
        assert!(ok);
        assert_eq!(String::from_utf8_lossy(&output), "Prompt ");
    }

    #[test]
    fn confirm_with_io_rejects_no_and_prints_abort() {
        let mut input = Cursor::new("n\n");
        let mut output = Vec::new();
        let ok = confirm_with_io("Prompt ", &mut input, &mut output).expect("confirm");
        assert!(!ok);
        assert_eq!(String::from_utf8_lossy(&output), "Prompt 🚫 Aborted\n");
    }

    #[test]
    fn confirm_with_io_rejects_empty_input() {
        let mut input = Cursor::new("\n");
        let mut output = Vec::new();
        let ok = confirm_with_io("Prompt ", &mut input, &mut output).expect("confirm");
        assert!(!ok);
        assert_eq!(String::from_utf8_lossy(&output), "Prompt 🚫 Aborted\n");
    }
}
