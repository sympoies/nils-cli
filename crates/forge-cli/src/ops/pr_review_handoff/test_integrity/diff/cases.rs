//! Row keys for removed tests: JavaScript titles.

/// Separates hunks inside the collected old/new line lists, so module nesting
/// and multi-line calls never continue across a gap in the visible source.
pub(super) const HUNK: &str = "\0hunk";

pub(super) fn visit_braces(
    line: &str,
    script_quotes: bool,
    mut visit: impl FnMut(usize, char),
) -> bool {
    let chars: Vec<_> = line.char_indices().collect();
    let (mut index, mut quote) = (0, None);
    while index < chars.len() {
        let (offset, ch) = chars[index];
        match (ch, quote) {
            ('\\', Some(_)) => index += 1,
            (ch, Some(delimiter)) if ch == delimiter => quote = None,
            (_, Some(_)) => {}
            ('"', None) => quote = Some(ch),
            ('\'' | '`', None) if script_quotes => quote = Some(ch),
            ('/', None) if chars.get(index + 1).is_some_and(|&(_, c)| c == '/') => break,
            ('\'', None) if chars.get(index + 2).is_some_and(|&(_, c)| c == '\'') => index += 2,
            ('{' | '}', None) => visit(offset, ch),
            _ => {}
        }
        index += 1;
    }
    quote.is_none()
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
pub(super) fn quoted(text: &str) -> Option<usize> {
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
