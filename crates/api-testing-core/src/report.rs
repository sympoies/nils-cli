use crate::markdown;

pub struct ReportHeader<'a> {
    pub report_date: &'a str,
    pub case_name: &'a str,
    pub generated_at: &'a str,
    pub endpoint_note: &'a str,
    pub result_note: &'a str,
    pub command_snippet: Option<&'a str>,
}

pub struct ReportBuilder {
    out: String,
}

impl ReportBuilder {
    pub fn new(header: ReportHeader<'_>) -> Self {
        let mut out = String::new();

        out.push_str(&markdown::heading(
            1,
            &format!("API Test Report ({})", header.report_date),
        ));
        out.push('\n');
        out.push_str(&markdown::heading(
            2,
            &format!("Test Case: {}", header.case_name),
        ));
        out.push('\n');

        if let Some(cmd) = header.command_snippet {
            out.push_str(&markdown::heading(2, "Command"));
            out.push('\n');
            out.push_str(&markdown::code_block("bash", cmd));
            out.push('\n');
        }

        out.push_str(&format!("Generated at: {}\n\n", header.generated_at));
        out.push_str(&format!("{}\n\n", header.endpoint_note));
        out.push_str(&format!("{}\n\n", header.result_note));

        Self { out }
    }

    pub fn push_section_heading(&mut self, title: &str) {
        self.out.push_str(&format!("### {title}\n\n"));
    }

    pub fn push_list_item(&mut self, item: &str) {
        self.out.push_str(&format!("- {item}\n"));
    }

    pub fn push_blank_line(&mut self) {
        self.out.push('\n');
    }

    pub fn push_code_section(
        &mut self,
        title: &str,
        lang: &str,
        body: &str,
        note: Option<&str>,
        trailing_blank: bool,
    ) {
        self.push_section_heading(title);
        if let Some(note) = note {
            let note = note.trim();
            if !note.is_empty() {
                self.out.push_str(note);
                self.out.push_str("\n\n");
            }
        }
        self.out.push_str(&markdown::code_block(lang, body));
        if trailing_blank {
            self.out.push('\n');
        }
    }

    pub fn finish(self) -> String {
        self.out
    }
}
