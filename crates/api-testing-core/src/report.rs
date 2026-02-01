use crate::Result;

pub fn render_markdown_report(_context: &serde_json::Value) -> Result<String> {
    anyhow::bail!("api-testing-core::report::render_markdown_report is not implemented yet");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markdown_report_is_unimplemented() {
        let err = render_markdown_report(&serde_json::json!({"run": 1})).unwrap_err();
        assert!(err
            .to_string()
            .contains("api-testing-core::report::render_markdown_report is not implemented"));
    }
}
