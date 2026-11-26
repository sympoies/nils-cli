use super::{cases, syntax::Language};

#[derive(Default)]
pub(super) struct Scopes {
    stack: Vec<(String, usize)>,
    depth: usize,
    header: Option<String>,
    describe: bool,
}

impl Scopes {
    pub(super) fn key(&self, name: &str) -> String {
        let mut key = String::new();
        for (scope, _) in &self.stack {
            key.push_str(scope);
            key.push_str("::");
        }
        key.push_str(name);
        key
    }

    pub(super) fn python_line(&mut self, code: &str) {
        if self.depth == 0 {
            let indentation =
                code.bytes()
                    .take_while(u8::is_ascii_whitespace)
                    .fold(0, |column, byte| {
                        if byte == b'\t' {
                            (column / 8 + 1) * 8
                        } else {
                            column + 1
                        }
                    });
            while self.stack.last().is_some_and(|(_, at)| *at >= indentation) {
                self.stack.pop();
            }
            if let Some(name) = code
                .trim_start()
                .strip_prefix("class ")
                .and_then(identifier)
            {
                self.stack.push((name.into(), indentation));
            }
        }
        for bracket in code.bytes() {
            match bracket {
                b'(' | b'[' | b'{' => self.depth += 1,
                b')' | b']' | b'}' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
        }
    }

    pub(super) fn enter<'a>(
        &mut self,
        mut raw: &'a str,
        mut code: &'a str,
        language: Language,
    ) -> (&'a str, &'a str) {
        loop {
            let trimmed = code.trim();
            let header = match language {
                Language::Rust => rust_scope(trimmed),
                Language::Java => java_scope(trimmed),
                Language::JavaScript => self.describe_scope(raw.trim_start(), trimmed),
                _ => None,
            };
            if header.is_some() {
                self.header = header;
            }
            if self.header.is_none() {
                return (raw, code);
            }
            let Some(open) = code
                .find(['{', '}', ';'])
                .filter(|&at| code.as_bytes()[at] == b'{')
            else {
                return (raw, code);
            };
            self.stack.push((self.header.take().unwrap(), self.depth));
            self.depth += 1;
            raw = &raw[open + 1..];
            code = &code[open + 1..];
        }
    }

    pub(super) fn finish_line(&mut self, code: &str) {
        for brace in code.bytes().filter(|b| matches!(b, b'{' | b'}' | b';')) {
            match brace {
                b'{' => self.depth += 1,
                b'}' => {
                    self.header = None;
                    self.depth = self.depth.saturating_sub(1);
                    while self.stack.last().is_some_and(|(_, at)| *at >= self.depth) {
                        self.stack.pop();
                    }
                }
                _ => self.header = None,
            }
        }
    }

    fn describe_scope(&mut self, raw: &str, code: &str) -> Option<String> {
        let opener = ["describe(", "describe.only(", "describe.skip("];
        let compact = compact(code);
        let started = opener.iter().any(|prefix| compact.starts_with(prefix));
        let title = if started {
            raw.split_once('(')?.1.trim_start()
        } else if self.describe {
            raw
        } else {
            return None;
        };
        self.describe = title.is_empty() || title.starts_with("//");
        let end = cases::quoted(title)?;
        Some(format!("describe({}", compact_label(&title[..end])))
    }
}

fn rust_scope(line: &str) -> Option<String> {
    let (_, declaration) = rust_attributes(line);
    let declaration = strip_rust_visibility(declaration);
    if let Some(rest) = declaration.strip_prefix("mod ") {
        if rest.split('{').next()?.contains(';') {
            return None;
        }
        return identifier(rest).map(str::to_string);
    }
    let header = declaration
        .strip_prefix("unsafe ")
        .unwrap_or(declaration)
        .strip_prefix("impl")?;
    if !header.starts_with([' ', '<', '\t']) {
        return None;
    }
    let header = strip_generics(header.trim_start())?.trim_start();
    let name = header
        .split(['{', ';'])
        .next()?
        .split(" where")
        .next()?
        .trim();
    (!name.is_empty()).then(|| name.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn java_scope(line: &str) -> Option<String> {
    let (_, declaration) = java_annotations(line);
    let mut words = declaration.split_whitespace();
    loop {
        match words.next()? {
            "public" | "protected" | "private" | "abstract" | "static" | "final" | "strictfp"
            | "sealed" | "non-sealed" => {}
            "class" => return identifier(words.next()?).map(str::to_string),
            _ => return None,
        }
    }
}

pub(super) fn identifier(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let first = text.chars().next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let end = text
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    Some(&text[..end])
}

fn strip_generics(text: &str) -> Option<&str> {
    if !text.starts_with('<') {
        return Some(text);
    }
    let mut depth = 0;
    for (offset, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[offset + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn strip_rust_visibility(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("pub ") {
        rest.trim_start()
    } else if let Some(rest) = text
        .strip_prefix("pub(")
        .and_then(|rest| rest.split_once(')'))
    {
        rest.1.trim_start()
    } else {
        text
    }
}

pub(super) fn rust_attributes(mut text: &str) -> (bool, &str) {
    let mut test = false;
    while let Some(attribute) = text
        .strip_prefix("#[")
        .and_then(|rest| rest.split_once(']'))
    {
        let name = attribute
            .0
            .split_once('(')
            .map_or(attribute.0, |(name, _)| name);
        test |= matches!(
            name.trim().rsplit("::").next(),
            Some("test" | "rstest" | "test_case")
        ) || compact(attribute.0) == "cfg(test)";
        text = attribute.1.trim_start();
    }
    (test, text)
}

pub(super) fn java_annotations(mut text: &str) -> (bool, &str) {
    let mut test = false;
    while let Some(annotation) = text.strip_prefix('@') {
        let end = annotation
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.')))
            .unwrap_or(annotation.len());
        if end == 0 {
            break;
        }
        test |= annotation[..end].rsplit('.').next() == Some("Test");
        text = annotation[end..].trim_start();
        if let Some(arguments) = text.strip_prefix('(') {
            let mut depth = 1;
            let end = arguments.char_indices().find_map(|(offset, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(offset + 1)
            });
            text = end.map_or("", |end| arguments[end..].trim_start());
        }
    }
    (test, text)
}

pub(super) fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compact_label(title: &str) -> String {
    // A label can appear inside a Markdown declaration row.
    use std::fmt::Write;
    let mut label = String::new();
    for c in title.chars() {
        if c.is_whitespace() || matches!(c, '%' | '|') {
            write!(label, "%{{{:X}}}", u32::from(c)).unwrap();
        } else {
            label.push(c);
        }
    }
    label
}
