//! Bounded display labels shared by hook projection and session records.

pub fn model_label(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:[]".contains(&b))
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || [
            "sk-",
            "ghp_",
            "gho_",
            "ghu_",
            "ghs_",
            "ghr_",
            "github_pat_",
            "glpat-",
            "hf_",
            "xoxb-",
            "xoxp-",
            "xoxa-",
            "xoxr-",
            "xoxs-",
            "eyj",
            "npm_",
            "akia",
            "aiza",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return None;
    }
    Some(value.to_string())
}

pub fn effort_label(value: &str) -> Option<String> {
    matches!(
        value,
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra" | "auto"
    )
    .then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    #[test]
    fn session_model_credentials_are_unknown() {
        for value in [
            "sk-example",
            "ghp_example",
            "gho_example",
            "ghu_example",
            "ghs_example",
            "ghr_example",
            "xoxp-example",
            "xoxa-example",
            "eyJexample.example.example",
        ] {
            assert_eq!(model_label(value), None);
        }
        assert_eq!(
            model_label("local-model:8b").as_deref(),
            Some("local-model:8b")
        );
    }
}
