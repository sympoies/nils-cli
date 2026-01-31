use crate::markdown;

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
    let mut out = String::new();

    out.push_str(&markdown::heading(
        1,
        &format!("API Test Report ({})", report.report_date),
    ));
    out.push('\n');
    out.push_str(&markdown::heading(
        2,
        &format!("Test Case: {}", report.case_name),
    ));
    out.push('\n');

    if let Some(cmd) = &report.command_snippet {
        out.push_str(&markdown::heading(2, "Command"));
        out.push('\n');
        out.push_str(&markdown::code_block("bash", cmd));
        out.push('\n');
    }

    out.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    out.push_str(&format!("{}\n\n", report.endpoint_note));
    out.push_str(&format!("{}\n\n", report.result_note));

    if !report.assertions.is_empty() {
        out.push_str("### Assertions\n\n");
        for a in &report.assertions {
            out.push_str(&format!("- {} ({})\n", a.label, a.state));
        }
        out.push('\n');
    }

    out.push_str("### Request\n\n");
    out.push_str(&markdown::code_block("json", &report.request_json));
    out.push('\n');

    out.push_str("### Response\n\n");
    out.push_str(&markdown::code_block(
        &report.response_lang,
        &report.response_body,
    ));
    out.push('\n');

    if let Some(stderr_note) = &report.stderr_note {
        if !stderr_note.is_empty() {
            out.push_str("### stderr\n\n");
            out.push_str(&markdown::code_block("text", stderr_note));
        }
    }

    out
}
