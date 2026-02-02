use crate::report::{ReportBuilder, ReportHeader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestReportAssertion {
    pub label: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestReport {
    pub report_date: String,
    pub case_name: String,
    pub generated_at: String,
    pub endpoint_note: String,
    pub result_note: String,
    pub command_snippet: Option<String>,
    pub assertions: Vec<RestReportAssertion>,
    pub request_json: String,
    pub response_lang: String,
    pub response_body: String,
    pub stderr_note: Option<String>,
}

pub fn render_rest_report_markdown(report: &RestReport) -> String {
    let header = ReportHeader {
        report_date: &report.report_date,
        case_name: &report.case_name,
        generated_at: &report.generated_at,
        endpoint_note: &report.endpoint_note,
        result_note: &report.result_note,
        command_snippet: report.command_snippet.as_deref(),
    };
    let mut builder = ReportBuilder::new(header);

    if !report.assertions.is_empty() {
        builder.push_section_heading("Assertions");
        for a in &report.assertions {
            builder.push_list_item(&format!("{} ({})", a.label, a.state));
        }
        builder.push_blank_line();
    }

    builder.push_code_section("Request", "json", &report.request_json, None, true);
    builder.push_code_section(
        "Response",
        &report.response_lang,
        &report.response_body,
        None,
        true,
    );

    if let Some(stderr_note) = &report.stderr_note {
        if !stderr_note.is_empty() {
            builder.push_code_section("stderr", "text", stderr_note, None, false);
        }
    }

    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn rest_report_renders_markdown_with_optional_sections() {
        let report = RestReport {
            report_date: "2026-02-01".to_string(),
            case_name: "Health".to_string(),
            generated_at: "2026-02-01T00:00:00Z".to_string(),
            endpoint_note: "Endpoint: http://localhost:8080/health".to_string(),
            result_note: "Result: OK".to_string(),
            command_snippet: Some("curl -sS http://localhost:8080/health".to_string()),
            assertions: vec![RestReportAssertion {
                label: "status".to_string(),
                state: "pass".to_string(),
            }],
            request_json: r#"{"method":"GET","path":"/health"}"#.to_string(),
            response_lang: "json".to_string(),
            response_body: r#"{"ok":true}"#.to_string(),
            stderr_note: Some("warning: retrying".to_string()),
        };

        let got = render_rest_report_markdown(&report);
        let expected = concat!(
            "# API Test Report (2026-02-01)\n",
            "\n",
            "## Test Case: Health\n",
            "\n",
            "## Command\n",
            "\n",
            "```bash\n",
            "curl -sS http://localhost:8080/health\n",
            "```\n",
            "\n",
            "Generated at: 2026-02-01T00:00:00Z\n",
            "\n",
            "Endpoint: http://localhost:8080/health\n",
            "\n",
            "Result: OK\n",
            "\n",
            "### Assertions\n",
            "\n",
            "- status (pass)\n",
            "\n",
            "### Request\n",
            "\n",
            "```json\n",
            "{\"method\":\"GET\",\"path\":\"/health\"}\n",
            "```\n",
            "\n",
            "### Response\n",
            "\n",
            "```json\n",
            "{\"ok\":true}\n",
            "```\n",
            "\n",
            "### stderr\n",
            "\n",
            "```text\n",
            "warning: retrying\n",
            "```\n"
        );
        assert_eq!(got, expected);
    }

    #[test]
    fn rest_report_omits_command_assertions_and_empty_stderr() {
        let report = RestReport {
            report_date: "2026-02-01".to_string(),
            case_name: "No-Options".to_string(),
            generated_at: "2026-02-01T00:00:00Z".to_string(),
            endpoint_note: "Endpoint: x".to_string(),
            result_note: "Result: y".to_string(),
            command_snippet: None,
            assertions: vec![],
            request_json: "{}".to_string(),
            response_lang: "text".to_string(),
            response_body: "ok".to_string(),
            stderr_note: Some("".to_string()),
        };

        let got = render_rest_report_markdown(&report);
        assert!(!got.contains("## Command\n"));
        assert!(!got.contains("### Assertions\n"));
        assert!(!got.contains("### stderr\n"));
    }
}
