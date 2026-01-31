use std::path::Path;

use anyhow::Context;

use crate::suite::results::SuiteRunResults;
use crate::Result;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn render_junit_xml(results: &SuiteRunResults) -> String {
    let suite_name = if results.suite.trim().is_empty() {
        "suite".to_string()
    } else {
        results.suite.clone()
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str(&format!(
        "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\">\n",
        xml_escape(&suite_name),
        results.summary.total,
        results.summary.failed,
        results.summary.skipped
    ));

    for case in &results.cases {
        let seconds = (case.duration_ms as f64) / 1000.0;
        out.push_str(&format!(
            "  <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">",
            xml_escape(&case.id),
            xml_escape(&case.case_type),
            seconds
        ));

        let message = case.message.clone().unwrap_or_default();
        let message_trim = message.trim();
        match case.status.as_str() {
            "skipped" => {
                out.push_str(&format!(
                    "<skipped message=\"{}\"/>",
                    xml_escape(message_trim)
                ));
            }
            "failed" => {
                let failure_message = if message_trim.is_empty() {
                    "failed"
                } else {
                    message_trim
                };
                out.push_str(&format!(
                    "<failure message=\"{}\">",
                    xml_escape(failure_message)
                ));
                let mut detail = String::new();
                if let Some(cmd) = &case.command {
                    detail.push_str(&format!("command: {cmd}\n"));
                }
                if let Some(p) = &case.stdout_file {
                    detail.push_str(&format!("stdoutFile: {p}\n"));
                }
                if let Some(p) = &case.stderr_file {
                    detail.push_str(&format!("stderrFile: {p}\n"));
                }
                out.push_str(&xml_escape(&detail));
                out.push_str("</failure>");
            }
            _ => {}
        }

        out.push_str("</testcase>\n");
    }

    out.push_str("</testsuite>\n");
    out
}

pub fn write_junit_file(results: &SuiteRunResults, path: &Path) -> Result<()> {
    let xml = render_junit_xml(results);
    let Some(parent) = path.parent() else {
        anyhow::bail!("invalid junit path: {}", path.display());
    };
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create directory: {}", parent.display()))?;
    std::fs::write(path, xml).with_context(|| format!("write junit file: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junit_emits_basic_structure() {
        let results = SuiteRunResults {
            version: 1,
            suite: "smoke".to_string(),
            suite_file: "tests/api/suites/smoke.suite.json".to_string(),
            run_id: "20260131-000000Z".to_string(),
            started_at: "2026-01-31T00:00:00Z".to_string(),
            finished_at: "2026-01-31T00:00:01Z".to_string(),
            output_dir: "out/api-test-runner/20260131-000000Z".to_string(),
            summary: crate::suite::results::SuiteRunSummary {
                total: 1,
                passed: 0,
                failed: 1,
                skipped: 0,
            },
            cases: vec![crate::suite::results::SuiteCaseResult {
                id: "case".to_string(),
                case_type: "rest".to_string(),
                status: "failed".to_string(),
                duration_ms: 10,
                tags: vec![],
                command: Some("api-rest call".to_string()),
                message: Some("rest_runner_failed".to_string()),
                assertions: None,
                stdout_file: Some("out/x.response.json".to_string()),
                stderr_file: Some("out/x.stderr.log".to_string()),
            }],
        };

        let xml = render_junit_xml(&results);
        assert!(xml.contains("<testsuite"));
        assert!(xml.contains("<testcase"));
        assert!(xml.contains("<failure"));
    }
}
