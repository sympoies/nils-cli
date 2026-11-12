//! Row keys for removed tests: Rust module paths and JavaScript titles.

/// Separates hunks inside the collected old/new line lists, so module nesting
/// and multi-line calls never continue across a gap in the visible source.
pub(super) const HUNK: &str = "\0hunk";

/// Rust `mod name { ... }` nesting visible in the diff hunk. A module whose
/// opening line is outside the hunk context cannot be named.
#[derive(Default)]
pub(super) struct Modules {
    stack: Vec<(String, i32)>,
    depth: i32,
}

impl Modules {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// `outer::inner::` for the current position, or an empty string.
    pub(super) fn prefix(&self) -> String {
        self.stack
            .iter()
            .map(|(name, _)| format!("{name}::"))
            .collect()
    }

    pub(super) fn feed(&mut self, line: &str) {
        if let Some(name) = module_open(line) {
            self.stack.push((name, self.depth));
        }
        let (opens, closes) = braces(line);
        self.depth += opens - closes;
        while self.stack.last().is_some_and(|(_, at)| *at >= self.depth) {
            self.stack.pop();
        }
    }
}

fn module_open(line: &str) -> Option<String> {
    let mut rest = line;
    while rest.starts_with("#[") {
        rest = rest[rest.find(']')? + 1..].trim_start();
    }
    if let Some(after) = rest.strip_prefix("pub") {
        let after = after.trim_start();
        rest = match after.strip_prefix('(') {
            Some(scope) => scope[scope.find(')')? + 1..].trim_start(),
            None => after,
        };
    }
    let rest = rest.strip_prefix("mod ")?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0 && rest[end..].trim_start().starts_with('{')).then(|| rest[..end].to_string())
}

/// Brace counts outside string/char literals and line comments.
fn braces(line: &str) -> (i32, i32) {
    let chars: Vec<char> = line.chars().collect();
    let (mut opens, mut closes, mut index, mut string) = (0, 0, 0, false);
    while index < chars.len() {
        match chars[index] {
            '\\' if string => index += 1,
            '"' => string = !string,
            '/' if !string && chars.get(index + 1) == Some(&'/') => break,
            '\'' if !string && chars.get(index + 2) == Some(&'\'') => index += 2,
            '{' if !string => opens += 1,
            '}' if !string => closes += 1,
            _ => {}
        }
        index += 1;
    }
    (opens, closes)
}

/// A JavaScript case whose title has not been seen yet.
pub(super) enum Pending {
    /// `it(` or `test(` with the title on a later line.
    Call(String),
    /// `it.each(...)` / ``it.each`...` `` table; the title follows the table.
    Each {
        head: String,
        text: String,
        lines: usize,
    },
}

pub(super) enum Step {
    /// The line completed the key and belongs to the case.
    Done(String),
    /// The case has no title; the line is ordinary source.
    Untitled(String),
    Wait(Pending),
}

impl Pending {
    /// Key to use when the title never appears.
    pub(super) fn untitled(self) -> String {
        match self {
            Self::Call(call) => call,
            Self::Each { head, .. } => format!("{head}.each("),
        }
    }

    pub(super) fn resume(self, compact: &str) -> Step {
        match self {
            Self::Call(call) => match quoted(compact) {
                Some(end) => Step::Done(format!("{call}{}", &compact[..end])),
                None => Step::Untitled(call),
            },
            Self::Each {
                head,
                mut text,
                lines,
            } => {
                text.push_str(compact);
                each_key(head, text, lines + 1)
            }
        }
    }
}

/// Length of the quoted title at the start of `text`; a backslash escapes the
/// next character, so `\'` does not end a single-quoted title.
fn quoted(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, quote) = chars
        .next()
        .filter(|(_, c)| matches!(c, '\'' | '"' | '`'))?;
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            c if c == quote => return Some(index + 1),
            _ => {}
        }
    }
    None
}

/// A table of cases is one row, named by the title that follows the table.
fn each_key(head: String, text: String, lines: usize) -> Step {
    let title = text.char_indices().find_map(|(index, _)| {
        let rest = text[index..]
            .strip_prefix(")(")
            .or_else(|| text[index..].strip_prefix("`("))?;
        quoted(rest).map(|end| &rest[..end])
    });
    match title {
        Some(title) => Step::Done(format!("{head}.each({title}")),
        None if lines >= 100 => Step::Untitled(format!("{head}.each(")),
        None => Step::Wait(Pending::Each { head, text, lines }),
    }
}

/// Key for a JavaScript `it`/`test` call, or the pending state when its title
/// is on a later line. `None` means the line is not a JavaScript case call.
pub(super) fn javascript(compact: &str) -> Option<Step> {
    for head in ["it", "test"] {
        let Some(rest) = compact
            .strip_prefix(head)
            .and_then(|r| r.strip_prefix(".each"))
        else {
            continue;
        };
        if rest.starts_with(['(', '`']) {
            return Some(each_key(head.into(), rest.into(), 1));
        }
    }
    let (call, rest) = compact.split_once('(')?;
    if !matches!(
        call,
        "it" | "test" | "it.skip" | "test.skip" | "it.only" | "test.only"
    ) {
        return None;
    }
    if rest.is_empty() {
        return Some(Step::Wait(Pending::Call(format!("{call}("))));
    }
    quoted(rest).map(|end| Step::Done(format!("{call}({}", &rest[..end])))
}
