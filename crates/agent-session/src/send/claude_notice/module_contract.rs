//! Load-time contract of the embedded Claude Code hooks module.
//!
//! External Claude Code 2.1.296 statically rejects a hooks module whose engine handle `$`
//! is used other than as `$.noun.event(...)` at a call site. Its loader reports, for example,
//! "$ itself is assigned (bound, passed, spread, returned or read); $ is always spelled
//! $.noun.event(...) at the call site". The module is not otherwise executed by any test, so
//! this is the only guard that it still loads.
use std::process::Command;

use pretty_assertions::assert_eq;

use super::PLUGIN;
use super::runtime::MODULE;

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Returns the 1-based line of every `$` that is neither a hook parameter nor the head of a
/// `$.noun.member` access. Strings, comments, regular expression literals and template text
/// are skipped; `${...}` template expressions are scanned as code.
fn dollar_misuse(src: &str) -> Vec<usize> {
    let chars: Vec<char> = src.chars().collect();
    let mut bad = Vec::new();
    let (mut i, mut line, mut prev) = (0, 1, ' ');
    let mut prev_word = String::new();
    // `true` marks a `{` that opened a template expression.
    let mut braces: Vec<bool> = Vec::new();
    let mut in_template = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
        }
        if in_template {
            match c {
                '\\' => i += 1,
                '`' => in_template = false,
                '$' if chars.get(i + 1) == Some(&'{') => {
                    braces.push(true);
                    in_template = false;
                    i += 1;
                }
                _ => {}
            }
            i += 1;
            continue;
        }
        match c {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    line += usize::from(chars[i] == '\n');
                    i += 1;
                }
                i += 2;
                continue;
            }
            '/' if "(,=:!&|?{};[".contains(prev) || prev_word == "return" => {
                let mut class = false;
                i += 1;
                while i < chars.len() && (class || chars[i] != '/') {
                    match chars[i] {
                        '\\' => i += 1,
                        '[' => class = true,
                        ']' => class = false,
                        _ => {}
                    }
                    i += 1;
                }
                prev = '/';
            }
            '\'' | '"' => {
                i += 1;
                while i < chars.len() && chars[i] != c {
                    i += usize::from(chars[i] == '\\');
                    i += 1;
                }
                prev = c;
            }
            '`' => {
                in_template = true;
                prev = c;
            }
            '{' => {
                braces.push(false);
                prev = c;
            }
            '}' => {
                in_template = braces.pop() == Some(true);
                prev = c;
            }
            c if is_ident(c) => {
                let start = i;
                while i < chars.len() && is_ident(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if word == "$" && !dollar_allowed(&chars, start, i) {
                    bad.push(line);
                }
                prev = 'a';
                prev_word = word;
                continue;
            }
            c if !c.is_whitespace() => prev = c,
            _ => {}
        }
        if !c.is_whitespace() {
            prev_word.clear();
        }
        i += 1;
    }
    bad
}

/// `$` is allowed as the first parameter of an arrow function, or followed by `.noun.member`.
fn dollar_allowed(chars: &[char], start: usize, end: usize) -> bool {
    let member = |from: usize| {
        let mut at = from;
        if chars.get(at) != Some(&'.') {
            return None;
        }
        at += 1;
        let begin = at;
        while at < chars.len() && is_ident(chars[at]) {
            at += 1;
        }
        (at > begin).then_some(at)
    };
    if member(end).and_then(member).is_some() {
        return true;
    }
    let before = chars[..start].iter().rev().find(|c| !c.is_whitespace());
    let params_end = chars[end..]
        .iter()
        .position(|c| !(is_ident(*c) || *c == ',' || c.is_whitespace()))
        .map(|offset| end + offset);
    before == Some(&'(')
        && params_end.is_some_and(|at| {
            chars[at] == ')'
                && chars[at + 1..].iter().find(|c| !c.is_whitespace()) == Some(&'=')
                && chars[at + 1..].iter().filter(|c| !c.is_whitespace()).nth(1) == Some(&'>')
        })
}

#[test]
fn embedded_module_uses_the_engine_handle_only_as_a_call_site() {
    assert_eq!(dollar_misuse(MODULE), Vec::<usize>::new());
}

#[test]
fn dollar_check_rejects_every_form_the_loader_rejects() {
    for source in [
        "let host; host = $;",
        "use($, 1);",
        "const all = [...$];",
        "const f = () => { return $; };",
        "const { fs } = $;",
        "const fs = $.fs;",
        "const w = $\n  .fs.write;",
    ] {
        assert_eq!(dollar_misuse(source).len(), 1, "{source}");
    }
    let accepted = "on('x', async ($, e, next) => { await $.fs.write(`${$.plugin.root}/a$`, '$'); \
        return /^[a-f0-9]{64}$/.test(`${e}`); /* $ */ }); // $";
    assert_eq!(dollar_misuse(accepted), Vec::<usize>::new());
}

/// Runs the pinned external loader's own check; skipped where that Claude Code build is absent.
#[test]
fn embedded_module_loads_in_the_pinned_claude_code() {
    let pinned = Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == "2.1.296 (Claude Code)"
        });
    if !pinned {
        eprintln!("skipped: Claude Code 2.1.296 is not installed");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".claude-plugin")).expect("manifest dir");
    std::fs::create_dir_all(dir.path().join("hooks")).expect("hooks dir");
    let manifest =
        format!(r#"{{"name":"{PLUGIN}","version":"1.0.0","description":"module load check"}}"#);
    for (path, text) in [
        (".claude-plugin/plugin.json", manifest.as_str()),
        ("hooks/hooks.json", r#"{"modules":["./register.js"]}"#),
        ("hooks/register.js", MODULE),
    ] {
        std::fs::write(dir.path().join(path), text).expect("write");
    }
    let output = Command::new("claude")
        .args(["plugin", "validate"])
        .arg(dir.path())
        .output()
        .expect("claude plugin validate");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
