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

pub fn canonicalize_table_cell(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_line_break_run = false;

    for ch in value.chars() {
        match ch {
            '\n' | '\r' => {
                if !in_line_break_run {
                    out.push(' ');
                    in_line_break_run = true;
                }
            }
            '|' => {
                out.push('/');
                in_line_break_run = false;
            }
            _ => {
                out.push(ch);
                in_line_break_run = false;
            }
        }
    }

    out
}

fn sort_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                let v = map.get(k).expect("key exists");
                out.insert(k.clone(), sort_json(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(sort_json).collect())
        }
        other => other.clone(),
    }
}

/// Format JSON similar to `jq -S .` (stable key order, pretty printed).
pub fn format_json_pretty_sorted(value: &serde_json::Value) -> Result<String, serde_json::Error> {
    let sorted = sort_json(value);
    serde_json::to_string_pretty(&sorted)
}

pub fn heading(level: u8, text: &str) -> String {
    let level = level.clamp(1, 6);
    format!("{} {}\n", "#".repeat(level.into()), text.trim())
}

pub fn code_block(lang: &str, body: &str) -> String {
    let mut out = String::new();
    out.push_str("```");
    out.push_str(lang.trim());
    out.push('\n');
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_table_cell, code_block, format_json_pretty_sorted, heading,
        markdown_payload_violations, validate_markdown_payload,
    };

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

    #[test]
    fn canonicalize_table_cell_normalizes_markdown_unsafe_chars() {
        let value = "A|B\r\nC\nD\rE";
        assert_eq!(canonicalize_table_cell(value), "A/B C D E");
    }

    #[test]
    fn canonicalize_table_cell_is_idempotent() {
        let first = canonicalize_table_cell("x|y\r\nz");
        let second = canonicalize_table_cell(&first);
        assert_eq!(first, second);
    }

    #[test]
    fn markdown_code_block_is_newline_stable() {
        assert_eq!(code_block("json", "{ }"), "```json\n{ }\n```\n");
        assert_eq!(code_block("json", "{ }\n"), "```json\n{ }\n```\n");
    }

    #[test]
    fn markdown_heading_trims_and_clamps_level() {
        assert_eq!(heading(1, " Title "), "# Title\n");
        assert_eq!(heading(9, "Title"), "###### Title\n");
    }

    #[test]
    fn json_format_sorts_keys_recursively() {
        let v = serde_json::json!({"b": 1, "a": {"d": 4, "c": 3}});
        let s = format_json_pretty_sorted(&v).expect("sorted json");
        assert_eq!(
            s,
            "{\n  \"a\": {\n    \"c\": 3,\n    \"d\": 4\n  },\n  \"b\": 1\n}"
        );
    }
}
