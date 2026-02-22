use std::path::Path;

use anyhow::Context;

use crate::Result;

fn strip_block_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut in_comment = false;

    while i < bytes.len() {
        if !in_comment {
            if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                in_comment = true;
                out.push(b' ');
                out.push(b' ');
                i += 2;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
            continue;
        }

        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            in_comment = false;
            out.push(b' ');
            out.push(b' ');
            i += 2;
            continue;
        }

        let b = bytes[i];
        if b == b'\n' || b == b'\r' {
            out.push(b);
        } else {
            out.push(b' ');
        }
        i += 1;
    }

    String::from_utf8(out).unwrap_or_default()
}

fn strip_strings(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum State {
        Normal,
        Triple,
        Double { escape: bool },
    }

    let mut state = State::Normal;

    while i < bytes.len() {
        match state {
            State::Normal => {
                if i + 2 < bytes.len()
                    && bytes[i] == b'"'
                    && bytes[i + 1] == b'"'
                    && bytes[i + 2] == b'"'
                {
                    state = State::Triple;
                    out.extend_from_slice(b"   ");
                    i += 3;
                    continue;
                }
                if bytes[i] == b'"' {
                    state = State::Double { escape: false };
                    out.push(b' ');
                    i += 1;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            State::Triple => {
                if i + 2 < bytes.len()
                    && bytes[i] == b'"'
                    && bytes[i + 1] == b'"'
                    && bytes[i + 2] == b'"'
                {
                    state = State::Normal;
                    out.extend_from_slice(b"   ");
                    i += 3;
                    continue;
                }
                let b = bytes[i];
                if b == b'\n' || b == b'\r' {
                    out.push(b);
                } else {
                    out.push(b' ');
                }
                i += 1;
            }
            State::Double { escape } => {
                let b = bytes[i];
                if escape {
                    if b == b'\n' || b == b'\r' {
                        out.push(b);
                    } else {
                        out.push(b' ');
                    }
                    state = State::Double { escape: false };
                    i += 1;
                    continue;
                }

                if b == b'\\' {
                    out.push(b' ');
                    state = State::Double { escape: true };
                    i += 1;
                    continue;
                }

                if b == b'"' {
                    out.push(b' ');
                    state = State::Normal;
                    i += 1;
                    continue;
                }

                if b == b'\n' || b == b'\r' {
                    out.push(b);
                } else {
                    out.push(b' ');
                }
                i += 1;
            }
        }
    }

    String::from_utf8(out).unwrap_or_default()
}

fn strip_line_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut in_comment = false;

    while i < bytes.len() {
        if !in_comment {
            if bytes[i] == b'#' {
                in_comment = true;
                out.push(b' ');
                i += 1;
                continue;
            }
            if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_comment = true;
                out.push(b' ');
                out.push(b' ');
                i += 2;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
            continue;
        }

        let b = bytes[i];
        if b == b'\n' || b == b'\r' {
            in_comment = false;
            out.push(b);
        } else {
            out.push(b' ');
        }
        i += 1;
    }

    String::from_utf8(out).unwrap_or_default()
}

pub fn operation_text_is_mutation(text: &str) -> bool {
    // Keep newlines, replace stripped ranges with spaces to prevent creating accidental tokens
    // (parity with prior `re.sub(..., " ", ...)` behavior).
    let cleaned = strip_line_comments(&strip_strings(&strip_block_comments(text)));

    for raw_line in cleaned.lines() {
        let line = raw_line.trim_start();
        if !line
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("mutation"))
        {
            continue;
        }

        let after = &line[8..];
        let boundary_ok = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        if !boundary_ok {
            continue;
        }

        let after_ws = after.trim_start();
        let Some(next) = after_ws.chars().next() else {
            continue;
        };

        let ok =
            next == '(' || next == '@' || next == '{' || next == '_' || next.is_ascii_alphabetic();
        if ok {
            return true;
        }
    }

    false
}

pub fn operation_file_is_mutation(path: &Path) -> Result<bool> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read GraphQL operation file: {}", path.display()))?;
    Ok(operation_text_is_mutation(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_detection_matches_expected_intent() {
        assert!(operation_text_is_mutation(
            r#"
query Q { ok }
mutation CreateUser($x: Int) { createUser(x: $x) { id } }
"#
        ));

        assert!(!operation_text_is_mutation("mutation: Mutation"));
        assert!(!operation_text_is_mutation(r#"# mutation { no }"#));
        assert!(!operation_text_is_mutation(r#""mutation { no }""#));
        assert!(!operation_text_is_mutation(
            r#"
query Example {
  foo(text: """
mutation { no }
""")
}
"#
        ));
        assert!(!operation_text_is_mutation(r#"// mutation { no }"#));
    }

    #[test]
    fn does_not_create_false_positive_when_comment_splits_keyword() {
        assert!(!operation_text_is_mutation("mu/*x*/tation { no }"));
    }
}
