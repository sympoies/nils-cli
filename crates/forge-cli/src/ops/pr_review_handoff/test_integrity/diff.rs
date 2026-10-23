//! Diff heuristics deliberately flag assertion edits even when stronger.
//! No claim of semantic implication: the independent reviewer resolves them.
use std::collections::{BTreeMap, BTreeSet};

use super::{ForgeError, incomplete, limit};

mod skip_calls;

#[derive(Debug, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Finding {
    pub path: String,
    pub kind: &'static str,
}

#[derive(Default)]
struct File {
    old: String,
    new: String,
    deleted: bool,
    before: Vec<String>,
    after: Vec<String>,
}

pub(super) fn detect(patch: &str) -> Result<Vec<Finding>, ForgeError> {
    if patch.trim().is_empty() {
        return Ok(Vec::new());
    }
    if patch.len() > 4 * 1024 * 1024 {
        return Err(limit("diff_bytes", "diff exceeds 4 MiB"));
    }
    if !patch.starts_with("diff --git ") {
        return Err(incomplete());
    }
    let mut findings = BTreeSet::new();
    let mut file = File::default();
    let mut remaining = (0usize, 0usize);
    let mut count = 0;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            if remaining != (0, 0) {
                return Err(incomplete());
            }
            if line.contains('"') {
                return Err(limit("path_bytes", "Git-quoted paths are unsupported"));
            }
            finish(&file, &mut findings);
            let names = line
                .strip_prefix("diff --git a/")
                .and_then(|s| s.split_once(" b/"))
                .ok_or_else(incomplete)?;
            file = File {
                old: path_name(names.0, "")?,
                new: path_name(names.1, "")?,
                ..File::default()
            };
            count += 1;
            // The comparison API bounds file lists; fail closed at its limit.
            if count >= 300 {
                return Err(limit("file_count", "comparison contains 300 or more files"));
            }
        } else if remaining != (0, 0) {
            let Some((prefix, content)) = line.split_at_checked(1) else {
                return Err(incomplete());
            };
            match prefix {
                "-" if remaining.0 > 0 => {
                    remaining.0 -= 1;
                    file.before.push(content.into());
                }
                "+" if remaining.1 > 0 => {
                    remaining.1 -= 1;
                    file.after.push(content.into());
                }
                " " if remaining.0 > 0 && remaining.1 > 0 => {
                    remaining.0 -= 1;
                    remaining.1 -= 1;
                    file.before.push(content.into());
                    file.after.push(content.into());
                }
                "\\" => {}
                _ => return Err(incomplete()),
            }
        } else if let Some(path) = line.strip_prefix("--- ") {
            file.old = path_name(path, "a/")?;
        } else if let Some(path) = line.strip_prefix("+++ ") {
            file.new = path_name(path, "b/")?;
        } else if line.starts_with("deleted file mode ") {
            file.deleted = true;
        } else if line.starts_with("@@ ") {
            let mut parts = line.split_whitespace();
            parts.next();
            remaining = (range(parts.next(), '-')?, range(parts.next(), '+')?);
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            // Binary assets are unrelated; source patches must remain inspectable.
            if source(&file.old) || source(&file.new) {
                return Err(incomplete());
            }
        } else if let Some(path) = line.strip_prefix("rename from ") {
            file.old = path_name(path, "")?;
        } else if let Some(path) = line.strip_prefix("rename to ") {
            file.new = path_name(path, "")?;
        } else if line.starts_with(['+', '-']) && !line.starts_with("---") {
            return Err(incomplete());
        }
    }
    if remaining != (0, 0) {
        return Err(incomplete());
    }
    finish(&file, &mut findings);
    Ok(findings.into_iter().collect())
}

fn path_name(path: &str, prefix: &str) -> Result<String, ForgeError> {
    let path = path.strip_prefix(prefix).unwrap_or(path);
    if !path.is_ascii()
        || path
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b'"' | b'|'))
    {
        return Err(limit(
            "path_bytes",
            "path requires quoting or contains unsupported non-ASCII, whitespace, control, quote or pipe bytes",
        ));
    }
    Ok(path.to_string())
}

fn range(value: Option<&str>, prefix: char) -> Result<usize, ForgeError> {
    let value = value
        .and_then(|v| v.strip_prefix(prefix))
        .ok_or_else(incomplete)?;
    let (start, length) = value.split_once(',').unwrap_or((value, "1"));
    start.parse::<usize>().map_err(|_| incomplete())?;
    length.parse::<usize>().map_err(|_| incomplete())
}

fn test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|p| matches!(p, "test" | "tests" | "__tests__"))
        || name.starts_with("test_")
        || name.ends_with("_test.go")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("Test.java")
}

fn source(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some(
            "rs" | "py"
                | "js"
                | "jsx"
                | "cjs"
                | "mjs"
                | "ts"
                | "tsx"
                | "cts"
                | "mts"
                | "go"
                | "java"
                | "kt"
                | "sh"
                | "bash"
                | "zsh"
                | "rb"
                | "cs"
                | "cpp"
                | "cc"
                | "c"
        )
    )
}

fn finish(file: &File, found: &mut BTreeSet<Finding>) {
    let renamed_test =
        !file.deleted && file.old != file.new && source(&file.old) && test_path(&file.old);
    if renamed_test {
        found.insert(Finding {
            path: file.old.clone(),
            kind: "deleted-test",
        });
    }
    let path = if file.deleted || file.new.is_empty() {
        &file.old
    } else {
        &file.new
    };
    if path.is_empty() || !source(path) {
        return;
    }
    let test = test_path(&file.old) || test_path(&file.new);
    let before = signals(&file.before, test);
    let after = signals(&file.after, test);
    let mut add = |kind| {
        found.insert(Finding {
            path: path.clone(),
            kind,
        });
    };
    if (file.deleted && test) || removed(&before.0, &after.0) {
        add("deleted-test");
    }
    if removed(&before.1, &after.1) {
        add("assertion-change");
    }
    if removed(&after.2, &before.2) {
        add("new-skip");
    }
}

type Counts = BTreeMap<String, usize>;
fn removed(old: &Counts, new: &Counts) -> bool {
    old.iter()
        .any(|(key, count)| *count > new.get(key).copied().unwrap_or(0))
}

fn signals(lines: &[String], test_path: bool) -> (Counts, Counts, Counts) {
    let mut cases = Counts::new();
    let mut assertions = Counts::new();
    let mut skips = Counts::new();
    let mut test_attribute = false;
    let mut assertion = String::new();
    let mut depth = 0i32;
    for raw in lines {
        let line = raw.trim();
        if line.starts_with(['/', '*'])
            || (line.starts_with('#') && !line.starts_with("#[") && !line.starts_with("#!["))
        {
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        let rust_test =
            line.starts_with("#[") && (compact.contains("test]") || compact.contains("test("));
        if rust_test || line.starts_with("@Test") {
            test_attribute = true;
        }
        if compact.contains("#[cfg(test)]") {
            test_attribute = true;
        }
        if test_attribute
            && (compact.starts_with("mod") || compact.contains("]mod"))
            && compact.ends_with(';')
        {
            *cases.entry(compact.clone()).or_default() += 1;
            test_attribute = false;
        }
        let function =
            line.starts_with("fn ") || line.starts_with("async fn ") || line.starts_with("pub fn ");
        let case = (function && (test_path || test_attribute))
            || (test_attribute && compact.contains('(') && compact.contains('{'))
            || line.starts_with("def test_")
            || line.starts_with("async def test_")
            || line.starts_with("func Test")
            || [
                "it(",
                "test(",
                "it.skip(",
                "test.skip(",
                "it.only(",
                "test.only(",
            ]
            .iter()
            .any(|p| compact.starts_with(p));
        if case {
            let key = compact
                .split(['{', ':'])
                .next()
                .unwrap_or(&compact)
                .to_string();
            *cases.entry(key).or_default() += 1;
            test_attribute = false;
        }
        if !assertion.is_empty()
            || compact.contains("assert")
            || compact.contains("expect(")
            || (test_path && compact.starts_with(".to"))
        {
            assertion.push_str(&compact);
            depth += compact
                .chars()
                .filter(|c| matches!(c, '(' | '[' | '{'))
                .count() as i32;
            depth -= compact
                .chars()
                .filter(|c| matches!(c, ')' | ']' | '}'))
                .count() as i32;
            if depth <= 0 {
                *assertions
                    .entry(std::mem::take(&mut assertion))
                    .or_default() += 1;
                depth = 0;
            }
        }
        if skip_calls::contains(line)
            || compact.contains("cfg(any())")
            || (compact.contains("cfg_attr(") && compact.contains("ignore"))
            || [
                "#[ignore",
                "pytest.mark.skip",
                "pytest.mark.xfail",
                "pytest.skip(",
                "pytest.xfail(",
                "unittest.skip",
                ".skipTest(",
                "unittest.expectedFailure",
                "@Ignore",
                "@Disabled",
                ".Skip(",
            ]
            .iter()
            .any(|p| compact.contains(p))
        {
            *skips.entry(compact).or_default() += 1;
        }
    }
    if !assertion.is_empty() {
        *assertions.entry(assertion).or_default() += 1;
    }
    (cases, assertions, skips)
}
