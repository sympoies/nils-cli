mod common;

use agent_docs::model::Context;

#[derive(Debug)]
struct ChecklistBegin<'a> {
    context: &'a str,
    mode: &'a str,
}

#[derive(Debug)]
struct ChecklistDoc<'a> {
    file_name: &'a str,
    status: &'a str,
    path: &'a str,
}

#[derive(Debug)]
struct ChecklistEnd<'a> {
    required: usize,
    present: usize,
    missing: usize,
    mode: &'a str,
    context: &'a str,
}

#[derive(Debug)]
struct ParsedChecklist<'a> {
    begin: ChecklistBegin<'a>,
    docs: Vec<ChecklistDoc<'a>>,
    end: ChecklistEnd<'a>,
}

#[test]
fn resolve_checklist_startup_is_machine_parseable_and_summary_consistent() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    let output = common::run_agent_docs_command(
        &workspace,
        &[
            "resolve",
            "--context",
            Context::Startup.as_str(),
            "--format",
            "checklist",
        ],
    );
    assert!(
        output.success(),
        "resolve startup checklist should succeed: code={} stderr=\n{}",
        output.exit_code,
        output.stderr
    );

    let parsed = parse_checklist(&output.stdout);
    assert_eq!(parsed.begin.context, Context::Startup.as_str());
    assert_eq!(parsed.begin.mode, "non-strict");
    assert_eq!(parsed.end.context, Context::Startup.as_str());
    assert_eq!(parsed.end.mode, "non-strict");
    assert_eq!(parsed.docs.len(), parsed.end.required);
    assert_eq!(
        parsed
            .docs
            .iter()
            .filter(|doc| doc.status == "present")
            .count(),
        parsed.end.present
    );
    assert_eq!(
        parsed
            .docs
            .iter()
            .filter(|doc| doc.status == "missing")
            .count(),
        parsed.end.missing
    );
    assert_eq!(
        parsed.end.required,
        parsed.end.present + parsed.end.missing,
        "required count should equal present + missing"
    );
    assert!(
        parsed
            .docs
            .iter()
            .all(|doc| !doc.file_name.is_empty() && !doc.path.is_empty()),
        "all checklist rows should include filename and path"
    );
}

#[test]
fn resolve_checklist_project_dev_is_machine_parseable_and_summary_consistent() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    let output = common::run_agent_docs_command(
        &workspace,
        &[
            "resolve",
            "--context",
            Context::ProjectDev.as_str(),
            "--format",
            "checklist",
        ],
    );
    assert!(
        output.success(),
        "resolve project-dev checklist should succeed: code={} stderr=\n{}",
        output.exit_code,
        output.stderr
    );

    let parsed = parse_checklist(&output.stdout);
    assert_eq!(parsed.begin.context, Context::ProjectDev.as_str());
    assert_eq!(parsed.begin.mode, "non-strict");
    assert_eq!(parsed.end.context, Context::ProjectDev.as_str());
    assert_eq!(parsed.end.mode, "non-strict");
    assert_eq!(parsed.docs.len(), parsed.end.required);
    assert_eq!(
        parsed
            .docs
            .iter()
            .filter(|doc| doc.status == "present")
            .count(),
        parsed.end.present
    );
    assert_eq!(
        parsed
            .docs
            .iter()
            .filter(|doc| doc.status == "missing")
            .count(),
        parsed.end.missing
    );
    assert!(
        parsed.docs.iter().all(|doc| doc.path.starts_with('/')),
        "checklist paths should be absolute in fixture runs"
    );
}

#[test]
fn resolve_checklist_output_is_deterministic_across_repeated_runs() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    let first = common::run_agent_docs_command(
        &workspace,
        &[
            "resolve",
            "--context",
            Context::Startup.as_str(),
            "--format",
            "checklist",
        ],
    );
    let second = common::run_agent_docs_command(
        &workspace,
        &[
            "resolve",
            "--context",
            Context::Startup.as_str(),
            "--format",
            "checklist",
        ],
    );

    assert!(first.success(), "first run failed: {}", first.stderr);
    assert!(second.success(), "second run failed: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "checklist output should be deterministic for stable fixture inputs"
    );
}

fn parse_checklist(output: &str) -> ParsedChecklist<'_> {
    let lines: Vec<&str> = output.lines().collect();
    assert!(
        lines.len() >= 2,
        "checklist output requires at least begin/end markers:\n{output}"
    );

    let begin = parse_begin_line(lines[0]);
    let end = parse_end_line(lines.last().expect("last line"));
    let docs = lines[1..lines.len() - 1]
        .iter()
        .map(|line| parse_doc_line(line))
        .collect();

    ParsedChecklist { begin, docs, end }
}

fn parse_begin_line(line: &str) -> ChecklistBegin<'_> {
    let payload = line
        .strip_prefix("REQUIRED_DOCS_BEGIN ")
        .expect("begin marker should start with REQUIRED_DOCS_BEGIN");
    let context = parse_kv(payload, "context").expect("begin marker should include context");
    let mode = parse_kv(payload, "mode").expect("begin marker should include mode");

    ChecklistBegin { context, mode }
}

fn parse_doc_line(line: &str) -> ChecklistDoc<'_> {
    let (file_name, remainder) = line
        .split_once(" status=")
        .expect("doc line should include status");
    let (status, path_payload) = remainder
        .split_once(" path=")
        .expect("doc line should include path");

    ChecklistDoc {
        file_name,
        status,
        path: path_payload,
    }
}

fn parse_end_line(line: &str) -> ChecklistEnd<'_> {
    let payload = line
        .strip_prefix("REQUIRED_DOCS_END ")
        .expect("end marker should start with REQUIRED_DOCS_END");

    let required = parse_kv(payload, "required")
        .expect("end marker should include required")
        .parse::<usize>()
        .expect("required should be usize");
    let present = parse_kv(payload, "present")
        .expect("end marker should include present")
        .parse::<usize>()
        .expect("present should be usize");
    let missing = parse_kv(payload, "missing")
        .expect("end marker should include missing")
        .parse::<usize>()
        .expect("missing should be usize");
    let mode = parse_kv(payload, "mode").expect("end marker should include mode");
    let context = parse_kv(payload, "context").expect("end marker should include context");

    ChecklistEnd {
        required,
        present,
        missing,
        mode,
        context,
    }
}

fn parse_kv<'a>(payload: &'a str, key: &str) -> Option<&'a str> {
    payload
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&format!("{key}=")))
}
