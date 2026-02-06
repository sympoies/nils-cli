use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiStripMode {
    CsiSgrOnly,
    CsiAnyTerminator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingleQuoteEscapeStyle {
    Backslash,
    DoubleQuoteBoundary,
}

pub fn quote_posix_single(input: &str) -> String {
    quote_posix_single_with_style(input, SingleQuoteEscapeStyle::Backslash)
}

pub fn quote_posix_single_with_style(input: &str, style: SingleQuoteEscapeStyle) -> String {
    if input.is_empty() {
        return "''".to_string();
    }

    let mut out = String::from("'");
    for ch in input.chars() {
        if ch == '\'' {
            match style {
                SingleQuoteEscapeStyle::Backslash => out.push_str("'\\''"),
                SingleQuoteEscapeStyle::DoubleQuoteBoundary => out.push_str("'\"'\"'"),
            }
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub fn strip_ansi(input: &str, mode: AnsiStripMode) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut copied_from = 0usize;
    let mut out: Option<String> = None;

    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            match mode {
                AnsiStripMode::CsiSgrOnly => {
                    while j < bytes.len() {
                        if bytes[j] == b'm' {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                }
                AnsiStripMode::CsiAnyTerminator => {
                    while j < bytes.len() {
                        let b = bytes[j];
                        j += 1;
                        if (0x40..=0x7e).contains(&b) {
                            break;
                        }
                    }
                }
            }

            let buffer = out.get_or_insert_with(|| String::with_capacity(input.len()));
            buffer.push_str(&input[copied_from..i]);
            copied_from = j;
            i = j;
            continue;
        }

        i += 1;
    }

    if let Some(mut buffer) = out {
        buffer.push_str(&input[copied_from..]);
        Cow::Owned(buffer)
    } else {
        Cow::Borrowed(input)
    }
}

#[cfg(test)]
mod tests {
    use super::SingleQuoteEscapeStyle;
    use super::{quote_posix_single, quote_posix_single_with_style, strip_ansi, AnsiStripMode};
    use std::borrow::Cow;

    #[test]
    fn quote_posix_single_uses_backslash_style() {
        assert_eq!(quote_posix_single("a'b"), "'a'\\''b'");
    }

    #[test]
    fn quote_posix_single_with_double_quote_boundary_style() {
        let out = quote_posix_single_with_style("a'b", SingleQuoteEscapeStyle::DoubleQuoteBoundary);
        assert_eq!(out, "'a'\"'\"'b'");
    }

    #[test]
    fn quote_posix_single_handles_empty_input() {
        assert_eq!(quote_posix_single(""), "''");
    }

    #[test]
    fn strip_ansi_sgr_removes_m_sequences() {
        let input = "\x1b[31mred\x1b[0m plain";
        assert_eq!(strip_ansi(input, AnsiStripMode::CsiSgrOnly), "red plain");
    }

    #[test]
    fn strip_ansi_any_terminator_removes_k_sequence() {
        let input = "a\x1b[2Kb";
        assert_eq!(strip_ansi(input, AnsiStripMode::CsiAnyTerminator), "ab");
    }

    #[test]
    fn strip_ansi_returns_borrowed_when_no_escape_found() {
        let input = "plain text";
        let out = strip_ansi(input, AnsiStripMode::CsiSgrOnly);
        assert!(matches!(out, Cow::Borrowed("plain text")));
    }
}
