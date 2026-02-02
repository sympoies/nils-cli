use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::Result;

fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let first_ok = first == '_' || first.is_ascii_alphabetic();
    if !first_ok {
        return false;
    }

    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn parse_assignment_line(line: &str) -> Option<(String, String)> {
    let line = line.trim_end_matches('\r');
    let mut line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    if let Some(rest) = line.strip_prefix("export") {
        if rest.starts_with(char::is_whitespace) {
            line = rest.trim();
        }
    }

    let (lhs, rhs) = line.split_once('=')?;
    let key = lhs.trim();
    if !is_valid_key(key) {
        return None;
    }

    let raw_value = rhs.trim();
    let value = if let Some(stripped) = parse_quoted_value(raw_value) {
        stripped
    } else {
        strip_inline_comment(raw_value).to_string()
    };

    Some((key.to_string(), value))
}

fn parse_quoted_value(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }

    let closing_index = value[1..].find(quote).map(|idx| idx + 1)?;
    let remainder = value[closing_index + 1..].trim_start();
    if !remainder.is_empty() && !remainder.starts_with('#') {
        return None;
    }

    Some(value[1..closing_index].to_string())
}

fn strip_inline_comment(value: &str) -> &str {
    let mut prev_was_space = false;
    for (idx, ch) in value.char_indices() {
        if ch == '#' && prev_was_space {
            return value[..idx].trim_end();
        }
        prev_was_space = ch.is_whitespace();
    }
    value.trim_end()
}

/// Read an env var from a list of `.env`-like files using the legacy "last assignment wins" semantics.
///
/// Parity notes:
/// - Lines are trimmed.
/// - Lines starting with `#` are ignored.
/// - Optional `export ` prefix is supported.
/// - Values wrapped in single or double quotes are unwrapped.
/// - Empty values are treated as "not set".
pub fn read_var_last_wins(key: &str, files: &[&Path]) -> Result<Option<String>> {
    let mut value: Option<String> = None;

    for file in files {
        if !file.is_file() {
            continue;
        }

        let f = std::fs::File::open(file)?;
        let reader = BufReader::new(f);
        for line in reader.lines() {
            let line = line?;
            let Some((found_key, found_value)) = parse_assignment_line(&line) else {
                continue;
            };
            if found_key == key {
                value = Some(found_value);
            }
        }
    }

    match value {
        Some(v) if !v.is_empty() => Ok(Some(v)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn env_file_read_var_handles_export_and_quotes() {
        let tmp = TempDir::new().expect("tmp");
        let f = tmp.path().join("a.env");
        write(
            &f,
            r#"
# comment
 export   FOO = "bar"
BAZ='qux'
NOPE=   plain
"#,
        );

        assert_eq!(
            read_var_last_wins("FOO", &[&f]).unwrap(),
            Some("bar".to_string())
        );
        assert_eq!(
            read_var_last_wins("BAZ", &[&f]).unwrap(),
            Some("qux".to_string())
        );
        assert_eq!(
            read_var_last_wins("NOPE", &[&f]).unwrap(),
            Some("plain".to_string())
        );
        assert_eq!(read_var_last_wins("MISSING", &[&f]).unwrap(), None);
    }

    #[test]
    fn env_file_read_var_last_wins_across_files_and_lines() {
        let tmp = TempDir::new().expect("tmp");
        let base = tmp.path().join("base.env");
        let local = tmp.path().join("local.env");
        write(&base, "A=1\nA=2\n");
        write(&local, "A=3\n");

        assert_eq!(
            read_var_last_wins("A", &[&base, &local]).unwrap(),
            Some("3".to_string())
        );
    }

    #[test]
    fn env_file_empty_value_clears_key() {
        let tmp = TempDir::new().expect("tmp");
        let base = tmp.path().join("base.env");
        let local = tmp.path().join("local.env");
        write(&base, "A=1\n");
        write(&local, "A=\n");

        assert_eq!(read_var_last_wins("A", &[&base, &local]).unwrap(), None);
    }

    #[test]
    fn env_file_inline_comments_only_strip_unquoted_values() {
        let tmp = TempDir::new().expect("tmp");
        let f = tmp.path().join("inline.env");
        write(
            &f,
            r#"
FOO=bar # comment
BAR="baz # keep"
BAZ='qux # keep'
QUX=keep#hash
"#,
        );

        assert_eq!(
            read_var_last_wins("FOO", &[&f]).unwrap(),
            Some("bar".to_string())
        );
        assert_eq!(
            read_var_last_wins("BAR", &[&f]).unwrap(),
            Some("baz # keep".to_string())
        );
        assert_eq!(
            read_var_last_wins("BAZ", &[&f]).unwrap(),
            Some("qux # keep".to_string())
        );
        assert_eq!(
            read_var_last_wins("QUX", &[&f]).unwrap(),
            Some("keep#hash".to_string())
        );
    }
}
