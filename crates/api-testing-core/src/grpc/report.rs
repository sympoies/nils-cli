use crate::report::{ReportBuilder, ReportHeader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcReportAssertion {
    pub label: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcReport {
    pub report_date: String,
    pub case_name: String,
    pub generated_at: String,
    pub endpoint_note: String,
    pub result_note: String,
    pub command_snippet: Option<String>,
    pub assertions: Vec<GrpcReportAssertion>,
    pub request_json: String,
    pub response_lang: String,
    pub response_body: String,
    pub stderr_note: Option<String>,
}

pub fn render_grpc_report_markdown(report: &GrpcReport) -> String {
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

    builder.push_code_section("gRPC Request", "json", &report.request_json, None, true);
    builder.push_code_section(
        "Response",
        &report.response_lang,
        &report.response_body,
        None,
        true,
    );

    if let Some(stderr_note) = &report.stderr_note
        && !stderr_note.is_empty()
    {
        builder.push_code_section("stderr", "text", stderr_note, None, false);
    }

    builder.finish()
}
