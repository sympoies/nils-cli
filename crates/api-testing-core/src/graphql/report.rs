use crate::report::{ReportBuilder, ReportHeader};

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
    let header = ReportHeader {
        report_date: &report.report_date,
        case_name: &report.case_name,
        generated_at: &report.generated_at,
        endpoint_note: &report.endpoint_note,
        result_note: &report.result_note,
        command_snippet: report.command_snippet.as_deref(),
    };
    let mut builder = ReportBuilder::new(header);

    builder.push_code_section(
        "GraphQL Operation",
        "graphql",
        &report.operation,
        None,
        true,
    );
    builder.push_code_section(
        "GraphQL Operation (Variables)",
        "json",
        &report.variables_json,
        report.variables_note.as_deref(),
        true,
    );
    builder.push_code_section(
        "Response",
        &report.response_lang,
        &report.response_body,
        report.response_note.as_deref(),
        false,
    );

    builder.finish()
}
