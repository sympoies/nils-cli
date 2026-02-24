use std::error::Error;
use std::fmt;

const LITERAL_ESCAPED_CONTROLS: [&str; 3] = [r"\n", r"\r", r"\t"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPayloadViolation {
    pub sequence: &'static str,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPayloadError {
    violations: Vec<MarkdownPayloadViolation>,
}

impl MarkdownPayloadError {
    pub fn violations(&self) -> &[MarkdownPayloadViolation] {
        &self.violations
    }
}

impl fmt::Display for MarkdownPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let details = self
            .violations
            .iter()
            .map(|entry| format!("{} ({})", entry.sequence, entry.count))
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "markdown payload contains literal escaped-control artifacts: {details}"
        )
    }
}

impl Error for MarkdownPayloadError {}

pub fn markdown_payload_violations(markdown: &str) -> Vec<MarkdownPayloadViolation> {
    let mut violations = Vec::new();

    for sequence in LITERAL_ESCAPED_CONTROLS {
        let count = markdown.match_indices(sequence).count();
        if count > 0 {
            violations.push(MarkdownPayloadViolation { sequence, count });
        }
    }

    violations
}

pub fn validate_markdown_payload(markdown: &str) -> Result<(), MarkdownPayloadError> {
    let violations = markdown_payload_violations(markdown);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(MarkdownPayloadError { violations })
    }
}

#[cfg(test)]
mod tests {
    use super::{markdown_payload_violations, validate_markdown_payload};

    #[test]
    fn markdown_payload_validator_accepts_real_control_chars() {
        let payload = "line one\nline two\tvalue\r\n";
        let result = validate_markdown_payload(payload);
        assert!(
            result.is_ok(),
            "unexpected markdown payload error: {result:?}"
        );
    }

    #[test]
    fn markdown_payload_validator_rejects_literal_escaped_controls() {
        let payload = r"line one\nline two\rline three\tvalue";
        let err = validate_markdown_payload(payload).expect_err("expected markdown payload error");

        assert_eq!(err.violations().len(), 3);
        assert!(
            err.to_string().contains(r"\n"),
            "expected escaped-newline mention in {:?}",
            err
        );
        assert!(
            err.to_string().contains(r"\r"),
            "expected escaped-return mention in {:?}",
            err
        );
        assert!(
            err.to_string().contains(r"\t"),
            "expected escaped-tab mention in {:?}",
            err
        );
    }

    #[test]
    fn markdown_payload_violations_reports_counts_per_sequence() {
        let payload = r"one\n two\n three\t";
        let violations = markdown_payload_violations(payload);

        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].sequence, r"\n");
        assert_eq!(violations[0].count, 2);
        assert_eq!(violations[1].sequence, r"\t");
        assert_eq!(violations[1].count, 1);
    }
}
