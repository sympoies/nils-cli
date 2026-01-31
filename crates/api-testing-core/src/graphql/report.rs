use crate::markdown;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlReport {
    pub report_date: String,
    pub case_name: String,
    pub generated_at: String,
    pub endpoint_note: String,
    pub result_note: String,
    pub command_snippet: Option<String>,
    pub operation: String,
    pub variables_note: Option<String>,
    pub variables_json: String,
    pub response_note: Option<String>,
    pub response_lang: String,
    pub response_body: String,
}

pub fn render_graphql_report_markdown(report: &GraphqlReport) -> String {
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

    out.push_str("### GraphQL Operation\n\n");
    out.push_str(&markdown::code_block("graphql", &report.operation));
    out.push('\n');

    out.push_str("### GraphQL Operation (Variables)\n\n");
    if let Some(note) = &report.variables_note {
        if !note.trim().is_empty() {
            out.push_str(note.trim());
            out.push_str("\n\n");
        }
    }
    out.push_str(&markdown::code_block("json", &report.variables_json));
    out.push('\n');

    out.push_str("### Response\n\n");
    if let Some(note) = &report.response_note {
        if !note.trim().is_empty() {
            out.push_str(note.trim());
            out.push_str("\n\n");
        }
    }
    out.push_str(&markdown::code_block(
        &report.response_lang,
        &report.response_body,
    ));

    out
}
