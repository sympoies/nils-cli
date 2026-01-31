use crate::Result;

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
pub fn format_json_pretty_sorted(value: &serde_json::Value) -> Result<String> {
    let sorted = sort_json(value);
    Ok(serde_json::to_string_pretty(&sorted)?)
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
    use super::*;
    use pretty_assertions::assert_eq;

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
        let s = format_json_pretty_sorted(&v).unwrap();
        assert_eq!(
            s,
            "{\n  \"a\": {\n    \"c\": 3,\n    \"d\": 4\n  },\n  \"b\": 1\n}"
        );
    }
}
