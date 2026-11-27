//! Diff heuristics deliberately flag assertion edits even when stronger.
//! No claim of semantic implication: the independent reviewer resolves them.
use std::collections::{BTreeMap, BTreeSet};

use super::{ForgeError, incomplete, limit};

mod cases;
mod coverage;
mod scopes;
mod skip_calls;
mod syntax;

use syntax::Language;

#[derive(Debug, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Finding {
    pub path: String,
    pub kind: String,
}

#[derive(Default)]
struct File {
    old: String,
    new: String,
    deleted: bool,
    change_is_non_additive: bool,
    additions: Vec<String>,
    before: Vec<String>,
    after: Vec<String>,
}

#[cfg(test)]
pub(crate) fn detect(patch: &str) -> Result<Vec<Finding>, ForgeError> {
    detect_with_images(patch, |_| Ok(None))
}

pub(crate) fn scoped_source(path: &str) -> bool {
    Language::of(path).has_scopes()
}

pub(super) fn detect_with_images(
    patch: &str,
    mut images: impl FnMut(&str) -> Result<Option<(String, String)>, ForgeError>,
) -> Result<Vec<Finding>, ForgeError> {
    let mut findings = BTreeSet::new();
    inspect(patch, |file| {
        let images = images(&file.new)?;
        finish(file, images.as_ref(), &mut findings);
        Ok(())
    })?;
    Ok(findings.into_iter().collect())
}

pub(crate) fn additive_coverage(
    patch: &str,
    fixture_review: impl Fn(&str) -> bool,
) -> Result<bool, ForgeError> {
    let mut all_additive = !patch.trim().is_empty();
    let mut findings = BTreeSet::new();
    inspect(patch, |file| {
        all_additive &= !file.change_is_non_additive
            && (coverage::file_adds_coverage(file)
                || (fixture_review(&file.new) && coverage::file_adds_fixture_coverage(file)));
        finish(file, None, &mut findings);
        Ok(())
    })?;
    Ok(all_additive && findings.is_empty())
}

fn inspect(
    patch: &str,
    mut inspect_file: impl FnMut(&File) -> Result<(), ForgeError>,
) -> Result<(), ForgeError> {
    if patch.trim().is_empty() {
        return Ok(());
    }
    if patch.len() > 4 * 1024 * 1024 {
        return Err(limit("diff_bytes", "diff exceeds 4 MiB"));
    }
    if !patch.starts_with("diff --git ") {
        return Err(incomplete());
    }
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
            if count > 0 {
                inspect_file(&file)?;
            }
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
                    file.change_is_non_additive = true;
                    file.before.push(content.into());
                }
                "+" if remaining.1 > 0 => {
                    remaining.1 -= 1;
                    file.additions.push(content.into());
                    file.after.push(content.into());
                }
                " " if remaining.0 > 0 && remaining.1 > 0 => {
                    if file
                        .additions
                        .last()
                        .is_some_and(|line| line != cases::HUNK)
                    {
                        file.additions.push(cases::HUNK.into());
                    }
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
            file.change_is_non_additive = true;
        } else if let Some(mode) = line.strip_prefix("new file mode ") {
            file.change_is_non_additive |= !matches!(mode, "100644" | "100755");
        } else if line.starts_with("old mode ") {
            file.change_is_non_additive = true;
        } else if line.starts_with("@@ ") {
            file.additions.push(cases::HUNK.into());
            file.before.push(cases::HUNK.into());
            file.after.push(cases::HUNK.into());
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
    inspect_file(&file)
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

pub(crate) const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "jsx", "cjs", "mjs", "ts", "tsx", "cts", "mts", "go", "java", "kt", "sh",
    "bash", "zsh", "rb", "cs", "cpp", "cc", "c",
];

pub(crate) fn source(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext))
}

fn finish(file: &File, images: Option<&(String, String)>, found: &mut BTreeSet<Finding>) {
    let renamed_test =
        !file.deleted && file.old != file.new && source(&file.old) && test_path(&file.old);
    if renamed_test {
        found.insert(Finding {
            path: file.old.clone(),
            kind: "deleted-test".into(),
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
    let old_language = Language::of(&file.old);
    let new_language = Language::of(&file.new);
    let before = signals(file.before.iter().map(String::as_str), old_language, test);
    let after = signals(file.after.iter().map(String::as_str), new_language, test);
    let full = images.map(|(before, after)| {
        (
            signals(before.lines(), old_language, test),
            signals(after.lines(), new_language, test),
        )
    });
    let (old_cases, new_cases, old_skips, new_skips) = match &full {
        Some((old, new)) => (&old.0, &new.0, &old.2, &new.2),
        None => (&before.0, &after.0, &before.2, &after.2),
    };
    let mut add = |kind: String| {
        found.insert(Finding {
            path: path.clone(),
            kind,
        });
    };
    if file.deleted && test {
        add("deleted-test".into());
    } else {
        // One row per removed test, so a declaration cannot cover its siblings.
        for key in removed_keys(old_cases, new_cases) {
            add(format!("deleted-test:{key}"));
        }
    }
    if removed(&before.1, &after.1) {
        add("assertion-change".into());
    }
    if removed(new_skips, old_skips) {
        add("new-skip".into());
    }
}

type Counts = BTreeMap<String, usize>;
fn removed_keys<'a>(old: &'a Counts, new: &'a Counts) -> impl Iterator<Item = &'a String> {
    old.iter()
        .filter(|(key, count)| **count > new.get(*key).copied().unwrap_or(0))
        .map(|(key, _)| key)
}

fn removed(old: &Counts, new: &Counts) -> bool {
    removed_keys(old, new).next().is_some()
}

fn signals<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    language: Language,
    test_path: bool,
) -> (Counts, Counts, Counts) {
    let mut tests = Counts::new();
    let mut assertions = Counts::new();
    let mut skips = Counts::new();
    let mut test_attribute = false;
    let mut assertion = String::new();
    let mut depth = 0i32;
    let mut scopes = scopes::Scopes::default();
    let mut lexical = syntax::LexicalState::default();
    let mut pending: Option<cases::Pending> = None;
    for raw in lines {
        if raw == cases::HUNK {
            if let Some(open) = pending.take() {
                *tests.entry(scopes.key(&open.untitled())).or_default() += 1;
            }
            scopes = scopes::Scopes::default();
            lexical = syntax::LexicalState::default();
            test_attribute = false;
            continue;
        }
        let masked = lexical.mask(raw, language);
        let code = masked.trim();
        let compact = scopes::compact(code);
        let original = scopes::compact(raw);
        let trimmed = raw.trim_start();
        let comment = code.is_empty()
            && if language == Language::Python {
                trimmed.starts_with('#')
            } else {
                ["//", "/*", "*"]
                    .iter()
                    .any(|marker| trimmed.starts_with(marker))
            };
        let mut consumed = false;
        if let Some(open) = pending.take() {
            if original.is_empty() || comment {
                pending = Some(open);
            } else {
                match open.resume(&original) {
                    cases::Step::Done(key) => {
                        *tests.entry(scopes.key(&key)).or_default() += 1;
                        consumed = true;
                    }
                    cases::Step::Untitled(key) => *tests.entry(scopes.key(&key)).or_default() += 1,
                    cases::Step::Wait(open) => {
                        pending = Some(open);
                        consumed = true;
                    }
                }
            }
        }
        let (case_source, case_code) = scopes.enter(raw, &masked, language);
        if !case_code.trim().is_empty() {
            if language == Language::Python {
                scopes.python_line(case_code);
            }
            if language == Language::JavaScript && !consumed {
                let source = scopes::compact(case_source);
                let code = scopes::compact(case_code);
                if javascript_case(&code, &source) {
                    match cases::javascript(&source) {
                        Some(cases::Step::Wait(open)) => pending = Some(open),
                        Some(cases::Step::Done(key) | cases::Step::Untitled(key)) => {
                            *tests.entry(scopes.key(&key)).or_default() += 1;
                        }
                        None => {
                            let key = source.split(['{', ':']).next().unwrap_or(&source);
                            *tests.entry(scopes.key(key)).or_default() += 1;
                        }
                    }
                }
            } else if let Some(name) =
                declaration(case_code.trim(), language, test_path, &mut test_attribute)
            {
                *tests.entry(scopes.key(&name)).or_default() += 1;
            }
        }
        if language.has_scopes() && language != Language::Python {
            scopes.finish_line(case_code);
        }
        if !comment
            && (!assertion.is_empty()
                || compact.contains("assert")
                || compact.contains("expect(")
                || (test_path && compact.starts_with(".to")))
        {
            assertion.push_str(&original);
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
        if skip_marker(code, &compact, language) {
            *skips.entry(original).or_default() += 1;
        }
    }
    if let Some(open) = pending {
        *tests.entry(scopes.key(&open.untitled())).or_default() += 1;
    }
    if !assertion.is_empty() {
        *assertions.entry(assertion).or_default() += 1;
    }
    (tests, assertions, skips)
}

fn javascript_case(code: &str, original: &str) -> bool {
    [
        "it(",
        "test(",
        "it.skip(",
        "test.skip(",
        "it.only(",
        "test.only(",
        "it.each(",
        "test.each(",
    ]
    .iter()
    .any(|prefix| code.starts_with(prefix))
        || (code.starts_with("it.each") && original.starts_with("it.each`"))
        || (code.starts_with("test.each") && original.starts_with("test.each`"))
}

fn declaration(
    code: &str,
    language: Language,
    test_path: bool,
    test_attribute: &mut bool,
) -> Option<String> {
    match language {
        Language::Rust => {
            let (attribute, declaration) = scopes::rust_attributes(code);
            *test_attribute |= attribute;
            if declaration.is_empty() {
                return None;
            }
            let attribute = std::mem::take(test_attribute);
            let declaration = scopes::strip_rust_visibility(declaration);
            if attribute && declaration.starts_with("mod ") && declaration.ends_with(';') {
                return Some(scopes::compact(declaration));
            }
            let declaration = declaration.strip_prefix("async ").unwrap_or(declaration);
            let name = declaration
                .strip_prefix("fn ")
                .and_then(scopes::identifier)?;
            (attribute || test_path).then(|| name.to_string())
        }
        Language::Java => {
            let (attribute, declaration) = scopes::java_annotations(code);
            *test_attribute |= attribute;
            if declaration.is_empty() {
                return None;
            }
            let attribute = std::mem::take(test_attribute);
            let (signature, _) = declaration.split_once('(')?;
            let name = scopes::identifier(signature.split_whitespace().last()?)?;
            attribute.then(|| name.to_string())
        }
        Language::Python => {
            let declaration = code.strip_prefix("async ").unwrap_or(code);
            let name = declaration
                .strip_prefix("def ")
                .and_then(scopes::identifier)?;
            name.starts_with("test_").then(|| name.to_string())
        }
        Language::Go => {
            let name = code.strip_prefix("func ").and_then(scopes::identifier)?;
            name.starts_with("Test").then(|| name.to_string())
        }
        _ => None,
    }
}

fn skip_marker(code: &str, compact: &str, language: Language) -> bool {
    match language {
        Language::Rust => {
            (compact.starts_with("#[") || compact.starts_with("#!["))
                && (compact.contains("cfg(any())")
                    || compact.starts_with("#[ignore")
                    || (compact.starts_with("#[cfg_attr(") && compact.contains("ignore")))
        }
        Language::Python => [
            "pytest.mark.skip",
            "pytest.mark.xfail",
            "pytest.skip(",
            "pytest.xfail(",
            "unittest.skip",
            ".skipTest(",
            "unittest.expectedFailure",
        ]
        .iter()
        .any(|marker| compact.contains(marker)),
        Language::JavaScript => skip_calls::contains(code),
        Language::Java => compact.starts_with("@Ignore") || compact.starts_with("@Disabled"),
        Language::Go => compact.contains(".Skip("),
        Language::Other => false,
    }
}
