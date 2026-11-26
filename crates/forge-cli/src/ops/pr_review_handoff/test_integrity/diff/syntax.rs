//! Hide literal and comment contents while retaining source offsets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Language {
    Rust,
    Python,
    JavaScript,
    Java,
    Go,
    Other,
}

impl Language {
    pub(super) fn of(path: &str) -> Self {
        match path.rsplit_once('.').map(|(_, extension)| extension) {
            Some("rs") => Self::Rust,
            Some("py") => Self::Python,
            Some("js" | "jsx" | "ts" | "tsx" | "cjs" | "mjs" | "cts" | "mts") => Self::JavaScript,
            Some("java") => Self::Java,
            Some("go") => Self::Go,
            _ => Self::Other,
        }
    }

    pub(super) fn has_scopes(self) -> bool {
        matches!(
            self,
            Self::Rust | Self::Python | Self::JavaScript | Self::Java
        )
    }
}

#[derive(Default)]
pub(super) struct LexicalState {
    literal: Option<Literal>,
    block_depth: usize,
}

enum Literal {
    Quoted {
        delimiter: &'static str,
        escapes: bool,
    },
    RustRaw(usize),
}

impl LexicalState {
    pub(super) fn mask(&mut self, line: &str, language: Language) -> String {
        let mut masked = vec![b' '; line.len()];
        let mut offset = 0;
        let mut escaped_at_end = false;
        while offset < line.len() {
            let rest = &line[offset..];
            let advance = if let Some(literal) = &self.literal {
                match literal {
                    Literal::Quoted { escapes: true, .. } if rest.starts_with('\\') => {
                        escaped_at_end = rest.len() == 1;
                        1 + rest[1..].chars().next().map_or(0, char::len_utf8)
                    }
                    Literal::Quoted { delimiter, .. } if rest.starts_with(delimiter) => {
                        let length = delimiter.len();
                        self.literal = None;
                        length
                    }
                    Literal::RustRaw(hashes)
                        if rest.starts_with('"')
                            && rest.as_bytes()[1..]
                                .iter()
                                .take(*hashes)
                                .filter(|&&b| b == b'#')
                                .count()
                                == *hashes =>
                    {
                        let length = 1 + hashes;
                        self.literal = None;
                        length
                    }
                    _ => char_len(rest),
                }
            } else if self.block_depth > 0 {
                if rest.starts_with("*/") {
                    self.block_depth -= 1;
                    2
                } else if language == Language::Rust && rest.starts_with("/*") {
                    self.block_depth += 1;
                    2
                } else {
                    char_len(rest)
                }
            } else if (language == Language::Python && rest.starts_with('#'))
                || (language != Language::Python && rest.starts_with("//"))
            {
                break;
            } else if language != Language::Python && rest.starts_with("/*") {
                self.block_depth = 1;
                2
            } else if let Some((literal, length)) = opening(rest, language) {
                self.literal = Some(literal);
                length
            } else {
                let length = char_len(rest);
                masked[offset..offset + length].copy_from_slice(&rest.as_bytes()[..length]);
                length
            };
            offset += advance;
        }
        if !escaped_at_end
            && language != Language::Rust
            && matches!(
                self.literal,
                Some(Literal::Quoted {
                    delimiter: "'" | "\"",
                    ..
                })
            )
        {
            self.literal = None;
        }
        String::from_utf8(masked).expect("mask preserves UTF-8")
    }
}

fn char_len(text: &str) -> usize {
    text.chars().next().unwrap().len_utf8()
}

fn opening(text: &str, language: Language) -> Option<(Literal, usize)> {
    if language == Language::Rust {
        let raw = text
            .strip_prefix("br")
            .or_else(|| text.strip_prefix("cr"))
            .or_else(|| text.strip_prefix('r'));
        if let Some(raw) = raw {
            let hashes = raw.bytes().take_while(|&b| b == b'#').count();
            if raw[hashes..].starts_with('"') {
                return Some((
                    Literal::RustRaw(hashes),
                    text.len() - raw.len() + hashes + 1,
                ));
            }
        }
    }
    let delimiter = match text.as_bytes()[0] {
        b'"' if matches!(language, Language::Python | Language::Java)
            && text.starts_with("\"\"\"") =>
        {
            "\"\"\""
        }
        b'\'' if language == Language::Python && text.starts_with("'''") => "'''",
        b'"' => "\"",
        b'\'' if language != Language::Rust || rust_char(text) => "'",
        b'`' if matches!(language, Language::JavaScript | Language::Go) => "`",
        _ => return None,
    };
    Some((
        Literal::Quoted {
            delimiter,
            escapes: !(language == Language::Go && delimiter == "`"),
        },
        delimiter.len(),
    ))
}

fn rust_char(text: &str) -> bool {
    // A Rust lifetime also begins with an apostrophe.
    let rest = &text[1..];
    rest.starts_with('\\') || rest.chars().nth(1) == Some('\'')
}
