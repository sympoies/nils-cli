//! Display metadata only. A valid marker never authenticates its author.
use std::fmt;

/// Validated v1 metadata, borrowed from the caller. Fields are self-asserted labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeEnvelope<'a> {
    kind: &'a str,
    source: &'a str,
    id: Option<&'a str>,
}

/// A parsed notice borrows the exact body slice; parsing never rewrites bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedNotice<'a> {
    pub metadata: NoticeEnvelope<'a>,
    pub body: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeError {
    InvalidField(&'static str),
    EmptyBody,
    DoubleHeader,
}

impl fmt::Display for NoticeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid notice {field}"),
            Self::EmptyBody => f.write_str("notice body must be nonempty"),
            Self::DoubleHeader => f.write_str("a notice cannot begin with another notice header"),
        }
    }
}
impl std::error::Error for NoticeError {}

impl<'a> NoticeEnvelope<'a> {
    pub fn new(kind: &'a str, source: &'a str, id: Option<&'a str>) -> Result<Self, NoticeError> {
        for (field, value, max) in [("kind", kind, 48), ("source", source, 64)] {
            if !token(value, max, true) {
                return Err(NoticeError::InvalidField(field));
            }
        }
        if let Some(id) = id
            && !token(id, 96, false)
        {
            return Err(NoticeError::InvalidField("id"));
        }
        Ok(Self { kind, source, id })
    }

    pub fn kind(self) -> &'a str {
        self.kind
    }
    pub fn source(self) -> &'a str {
        self.source
    }
    pub fn id(self) -> Option<&'a str> {
        self.id
    }

    /// Canonical LF separators; body bytes, including CRLF and trailing whitespace, are retained.
    pub fn encode(self, body: &str) -> Result<String, NoticeError> {
        validate_body(body)?;
        let mut header = format!(
            "<!-- agent-notice:v1 kind={} source={}",
            self.kind, self.source
        );
        if let Some(id) = self.id {
            header.push_str(&format!(" id={id}"));
        }
        Ok(format!("{header} -->\n\n{body}"))
    }
}

fn token(value: &str, max: usize, lowercase: bool) -> bool {
    let alpha = |b: u8| {
        if lowercase {
            b.is_ascii_lowercase()
        } else {
            b.is_ascii_alphanumeric()
        }
    };
    let part = |b: u8| b.is_ascii_digit() || alpha(b);
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > max || !alpha(bytes[0]) {
        return false;
    }
    let mut separator = false;
    for &b in &bytes[1..] {
        if part(b) {
            separator = false;
        } else if matches!(b, b'.' | b'_' | b'-') && !separator {
            separator = true;
        } else {
            return false;
        }
    }
    !separator
}

fn validate_body(body: &str) -> Result<(), NoticeError> {
    if body.is_empty() {
        return Err(NoticeError::EmptyBody);
    }
    if body.starts_with("<!-- agent-notice:") {
        return Err(NoticeError::DoubleHeader);
    }
    Ok(())
}

/// Parse only column zero of logical message line one. Provider wrappers must
/// be normalized by a provider-aware caller before this function is used.
/// Unknown versions and all malformed input return `None` and remain ordinary text.
pub fn parse_notice(input: &str) -> Option<ParsedNotice<'_>> {
    let (header, rest) = input.split_once('\n')?;
    let header = header.strip_suffix('\r').unwrap_or(header);
    if !header.is_ascii() || header.len() > 512 {
        return None;
    }
    let fields = header
        .strip_prefix("<!-- agent-notice:v1 ")?
        .strip_suffix(" -->")?;
    let mut fields = fields.split(' ');
    let kind = fields.next()?.strip_prefix("kind=")?;
    let source = fields.next()?.strip_prefix("source=")?;
    let id = match fields.next() {
        Some(field) => Some(field.strip_prefix("id=")?),
        None => None,
    };
    if fields.next().is_some() {
        return None;
    }
    let metadata = NoticeEnvelope::new(kind, source, id).ok()?;
    let body = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .unwrap_or(rest);
    validate_body(body).ok()?;
    Some(ParsedNotice { metadata, body })
}
