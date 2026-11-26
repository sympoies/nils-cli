//! Conservative boundaries for test coverage inserted in an existing file.
use std::collections::BTreeSet;

use super::{File, Language, cases, signals};

pub(super) fn file_adds_coverage(file: &File) -> bool {
    let (cases, assertions, _) = signals(
        file.additions.iter().map(String::as_str),
        Language::of(&file.new),
        false,
    );
    if file.old == "/dev/null" {
        return !cases.is_empty();
    }
    if cases.is_empty() && assertions.is_empty() {
        return false;
    }

    let extension = file.new.rsplit('.').next().unwrap_or("");
    let mut insertions = file.additions.split(|line| line == cases::HUNK).peekable();
    while let Some(lines) = insertions.next() {
        if coverage_insertion(lines, extension) {
            continue;
        }
        // Any following old context contributes an end marker. Appended
        // Python definitions can close by indentation at the end of the file.
        if extension != "py" || insertions.peek().is_some() || !python_append(lines, &file.before) {
            return false;
        }
    }
    true
}

fn coverage_insertion(added: &[String], extension: &str) -> bool {
    let mut lines = added
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty());
    while let Some(line) = lines.next() {
        if standalone_assertion(line) {
            continue;
        }
        let (body, suffix) = match extension {
            "rs" => (rust_body(line, &mut lines), ""),
            "go" => (line.strip_prefix("func Test").and_then(signature_body), ""),
            "java" => (
                annotated_signature(line, "@Test", &mut lines).and_then(signature_body),
                "",
            ),
            "js" | "jsx" | "ts" | "tsx" | "cjs" | "mjs" | "cts" | "mts" => {
                (javascript_body(line), ")")
            }
            _ => return false,
        };
        if !body
            .is_some_and(|body| complete_braced_case(body, &mut lines, suffix, extension != "rs"))
        {
            return false;
        }
    }
    true
}

fn annotated_signature<'a>(
    line: &'a str,
    attribute: &str,
    lines: &mut impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let rest = line.strip_prefix(attribute)?;
    if rest.is_empty() {
        lines.next()
    } else {
        rest.strip_prefix(' ').map(str::trim_start)
    }
}

fn signature_body(signature: &str) -> Option<&str> {
    let (signature, body) = signature.split_once('{')?;
    (signature.contains('(') && signature.contains(')')).then_some(body)
}

fn rust_body<'a>(line: &'a str, lines: &mut impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let signature = annotated_signature(line, "#[test]", lines)?.strip_prefix("fn ")?;
    let (name, body) = signature.split_once('{')?;
    let name = name.trim_end().strip_suffix("()")?;
    (!name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')).then_some(body)
}

fn javascript_body(line: &str) -> Option<&str> {
    let args = line
        .strip_prefix("it(")
        .or_else(|| line.strip_prefix("test("))?
        .trim_start();
    let title_end = cases::quoted(args)?;
    let callback = args[title_end..]
        .trim_start()
        .strip_prefix(',')?
        .trim_start();
    let callback = callback.strip_prefix("async ").unwrap_or(callback);
    let callback = callback
        .strip_prefix("()")?
        .trim_start()
        .strip_prefix("=>")?
        .trim_start();
    callback.strip_prefix('{')
}

fn complete_braced_case<'a>(
    body: &'a str,
    lines: &mut impl Iterator<Item = &'a str>,
    suffix: &str,
    script_quotes: bool,
) -> bool {
    // Closing inside the insertion gives the new case its own body.
    let mut depth = 1;
    for line in std::iter::once(body).chain(lines) {
        if ["/*", "*/", "r#"]
            .iter()
            .any(|marker| line.contains(marker))
        {
            return false;
        }
        let mut end = None;
        let complete_line = cases::visit_braces(line, script_quotes, |offset, brace| {
            if end.is_none() {
                depth += if brace == '{' { 1 } else { -1 };
                if depth == 0 {
                    end = Some(offset + 1);
                }
            }
        });
        if !complete_line {
            return false;
        }
        if let Some(end) = end {
            let rest = line[end..]
                .split_once("//")
                .map_or(&line[end..], |(code, _)| code);
            let rest = rest.trim();
            return rest == suffix
                || (!suffix.is_empty() && rest.strip_suffix(';') == Some(suffix));
        }
    }
    false
}

fn python_test_name(line: &str) -> Option<&str> {
    let signature = line
        .strip_prefix("def ")
        .or_else(|| line.strip_prefix("async def "))?;
    let (name, _) = signature.split_once('(')?;
    let name = name.trim_end();
    (name.starts_with("test_") && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
        .then_some(name)
}

fn python_append<'a>(added: &'a [String], before: &'a [String]) -> bool {
    let mut names: BTreeSet<_> = before
        .iter()
        .filter_map(|line| python_test_name(line))
        .collect();
    let mut has_test = false;
    let mut has_body = false;
    for line in added
        .iter()
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
    {
        if line.starts_with([' ', '\t']) {
            if !has_test {
                return false;
            }
            has_body = true;
        } else {
            if (has_test && !has_body) || !line.ends_with(':') {
                return false;
            }
            let Some(name) = python_test_name(line) else {
                return false;
            };
            if !names.insert(name) {
                return false;
            }
            has_test = true;
            has_body = false;
        }
    }
    has_test && has_body
}

fn standalone_assertion(line: &str) -> bool {
    let line = line.strip_suffix(';').unwrap_or(line);
    if line.contains([';', '{', '}', '\\']) {
        return false;
    }
    if let Some(condition) = line.strip_prefix("assert ") {
        return !condition.trim().is_empty();
    }
    line.ends_with(')')
        && line.split_once('(').is_some_and(|(name, _)| {
            matches!(
                name.trim_end(),
                "assert"
                    | "assert!"
                    | "assert_eq!"
                    | "assert_ne!"
                    | "debug_assert!"
                    | "debug_assert_eq!"
                    | "debug_assert_ne!"
                    | "expect"
            )
        })
}
