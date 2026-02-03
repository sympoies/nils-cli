use std::io::Write;
use std::path::{Path, PathBuf};

use api_testing_core::{env_file, Result};

pub(crate) fn trim_non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub(crate) fn bool_from_env(
    raw: Option<String>,
    name: &str,
    default: bool,
    stderr: &mut dyn Write,
) -> bool {
    let raw = raw.unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return default;
    }
    match raw.to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => {
            let _ = writeln!(
                stderr,
                "api-rest: warning: {name} must be true|false (got: {raw}); treating as false"
            );
            false
        }
    }
}

pub(crate) fn parse_u64_default(raw: Option<String>, default: u64, min: u64) -> u64 {
    let raw = raw.unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return default;
    }
    if !raw.chars().all(|c| c.is_ascii_digit()) {
        return default;
    }
    let Ok(v) = raw.parse::<u64>() else {
        return default;
    };
    v.max(min)
}

pub(crate) fn to_env_key(s: &str) -> String {
    env_file::normalize_env_key(s)
}

pub(crate) fn slugify(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric();
        if ok {
            out.push(c);
            prev_dash = false;
            continue;
        }
        if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

pub(crate) fn maybe_relpath(path: &Path, base: &Path) -> String {
    if path == base {
        return ".".to_string();
    }

    if let Ok(stripped) = path.strip_prefix(base) {
        let s = stripped.to_string_lossy();
        if s.is_empty() {
            return ".".to_string();
        }
        return s.to_string();
    }

    path.to_string_lossy().to_string()
}

pub(crate) fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub(crate) fn list_available_suffixes(file: &Path, prefix: &str) -> Vec<String> {
    if !file.is_file() {
        return Vec::new();
    }

    let Ok(content) = std::fs::read_to_string(file) else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end_matches('\r');
        let mut line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export") {
            if rest.starts_with(char::is_whitespace) {
                line = rest.trim();
            }
        }

        let Some((lhs, _rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let Some(suffix) = key.strip_prefix(prefix) else {
            continue;
        };
        if suffix.is_empty()
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        out.push(suffix.to_ascii_lowercase());
    }

    out.sort();
    out.dedup();
    out
}

pub(crate) fn find_git_root(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = start_dir;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

pub(crate) fn history_timestamp_now() -> Result<String> {
    let format = time::format_description::parse(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory][offset_minute]",
    )?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

pub(crate) fn report_stamp_now() -> Result<String> {
    let format = time::format_description::parse("[year][month][day]-[hour][minute]")?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

pub(crate) fn report_date_now() -> Result<String> {
    let format = time::format_description::parse("[year]-[month]-[day]")?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::test_support::{write_file, write_json};

    #[test]
    fn bool_from_env_parses_and_warns() {
        let mut stderr = Vec::new();
        let got = bool_from_env(Some("true".to_string()), "REST_FOO", false, &mut stderr);
        assert_eq!(got, true);

        let mut stderr = Vec::new();
        let got = bool_from_env(Some("nope".to_string()), "REST_FOO", true, &mut stderr);
        assert_eq!(got, false);
        let msg = String::from_utf8_lossy(&stderr);
        assert!(msg.contains("REST_FOO must be true|false"));
    }

    #[test]
    fn parse_u64_default_enforces_min() {
        assert_eq!(parse_u64_default(Some("".to_string()), 10, 1), 10);
        assert_eq!(parse_u64_default(Some("abc".to_string()), 10, 1), 10);
        assert_eq!(parse_u64_default(Some("0".to_string()), 10, 1), 1);
    }

    #[test]
    fn to_env_key_and_slugify_normalize() {
        assert_eq!(to_env_key("prod-us"), "PROD_US");
        assert_eq!(to_env_key("  foo@@bar  "), "FOO_BAR");
        assert_eq!(slugify("Hello, world!"), "hello-world");
        assert_eq!(slugify("  ___ "), "");
    }

    #[test]
    fn maybe_relpath_and_shell_quote() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        assert_eq!(maybe_relpath(root, root), ".");

        let child = root.join("a/b");
        std::fs::create_dir_all(&child).unwrap();
        assert_eq!(maybe_relpath(&child, root), "a/b");

        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn list_available_suffixes_parses_and_sorts() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("endpoints.env");
        write_file(
            &file,
            "export REST_URL_PROD=http://prod\nREST_URL_DEV=http://dev\nREST_URL_=bad\nREST_URL_FOO-BAR=http://x\nREST_URL_TEST=http://t\nREST_URL_TEST=http://t2\n",
        );

        let suffixes = list_available_suffixes(&file, "REST_URL_");
        assert_eq!(suffixes, vec!["dev", "prod", "test"]);
    }

    #[test]
    fn report_dates_are_formatted() {
        let stamp = report_stamp_now().unwrap();
        assert_eq!(stamp.len(), 13, "stamp={stamp}");

        let date = report_date_now().unwrap();
        assert_eq!(date.len(), 10, "date={date}");
        assert!(date.contains('-'));
    }

    #[test]
    fn history_timestamp_is_not_empty() {
        let stamp = history_timestamp_now().unwrap();
        assert!(!stamp.is_empty());
        assert!(stamp.contains('T'));
    }

    #[test]
    fn write_json_helper_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("example.json");
        write_json(&path, &serde_json::json!({"ok": true}));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"ok\""));
    }
}
