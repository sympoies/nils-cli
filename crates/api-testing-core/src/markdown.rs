use crate::Result;
use nils_common::markdown as common_markdown;

/// Format JSON similar to `jq -S .` (stable key order, pretty printed).
pub fn format_json_pretty_sorted(value: &serde_json::Value) -> Result<String> {
    Ok(common_markdown::format_json_pretty_sorted(value)?)
}

pub fn heading(level: u8, text: &str) -> String {
    common_markdown::heading(level, text)
}

pub fn code_block(lang: &str, body: &str) -> String {
    common_markdown::code_block(lang, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn json_format_sorts_keys_recursively() {
        let v = serde_json::json!({"b": 1, "a": {"d": 4, "c": 3}});
        let s = format_json_pretty_sorted(&v).unwrap();
        assert_eq!(
            s,
            "{\n  \"a\": {\n    \"c\": 3,\n    \"d\": 4\n  },\n  \"b\": 1\n}"
        );
    }
}
