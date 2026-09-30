//! Conformance of `forge_cli::tracker` against the vendored tracker row
//! grammar corpus (`tests/fixtures/tracker-row-grammar/`, copied unchanged from
//! the `agent-runtime-kit` repository, which owns the grammar).
//!
//! Every `valid/*` body must reproduce its expected rows and graph with no
//! finding; every `invalid/*` body must report exactly its expected findings,
//! compared as an unordered collection. The corpus is read from disk as bytes
//! so a new pair is picked up without touching this file.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use forge_cli::tracker::{self, Finding, Row};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const CODES: [&str; 6] = [
    "malformed-row",
    "duplicate-id",
    "unknown-dependency",
    "self-dependency",
    "cycle",
    "stale-graph",
];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tracker-row-grammar")
}

fn bodies(kind: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus().join(kind))
        .unwrap_or_else(|e| panic!("read corpus {kind}: {e}"))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    paths
}

/// Bytes, not a lossy or line-normalising read: some bodies depend on them.
fn body_of(path: &Path) -> String {
    String::from_utf8(fs::read(path).expect("read body")).expect("utf-8 body")
}

fn expected(path: &Path) -> Value {
    let raw = fs::read_to_string(path.with_extension("json")).expect("read expectation");
    serde_json::from_str(&raw).expect("expectation json")
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn row_json(row: &Row) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "ref": row.reference.as_ref().map(|r| json!({
            "owner": r.owner,
            "repo": r.repo,
            "number": r.number,
        })),
        "notes": row.notes,
        "after": row.after,
        "done": row.done,
        "phase": row.phase,
    })
}

fn finding_json(finding: &Finding) -> Value {
    json!({
        "code": finding.code.as_str(),
        "line": finding.line,
        "ids": finding.ids,
    })
}

/// Findings carry no order: sort both sides by their serialized form.
fn unordered(findings: &[Value]) -> Vec<String> {
    let mut keys: Vec<String> = findings
        .iter()
        .map(|f| format!("{}|{}|{}", f["code"], f["line"], f["ids"]))
        .collect();
    keys.sort();
    keys
}

#[test]
fn corpus_is_paired_and_covers_every_finding_code() {
    let mut covered = BTreeSet::new();
    for kind in ["valid", "invalid"] {
        let paths = bodies(kind);
        assert!(paths.len() >= 3, "{kind} corpus is too small: {paths:?}");
        for path in &paths {
            assert!(
                path.with_extension("json").is_file(),
                "{} has no expectation",
                name(path)
            );
        }
    }
    for path in bodies("invalid") {
        for finding in expected(&path)["findings"].as_array().expect("findings") {
            covered.insert(finding["code"].as_str().expect("code").to_string());
        }
    }
    let all: BTreeSet<String> = CODES.iter().map(|code| code.to_string()).collect();
    assert_eq!(covered, all);
}

#[test]
fn vendored_corpus_keeps_the_bytes_it_depends_on() {
    let edge = body_of(&corpus().join("valid/edge-cases.md"));
    assert_eq!(edge.matches('\u{a0}').count(), 2, "no-break spaces");
    assert_eq!(edge.matches('\t').count(), 1, "tab in an after list");
    let unclosed = body_of(&corpus().join("invalid/stale-graph--unclosed.md"));
    assert!(unclosed.ends_with("  A1 --> A2"), "no final line feed");
}

#[test]
fn valid_bodies_reproduce_their_rows_and_graph_without_findings() {
    for path in bodies("valid") {
        let body = body_of(&path);
        let want = expected(&path);
        let report = tracker::lint(&body);
        let findings: Vec<Value> = report.findings.iter().map(finding_json).collect();
        assert_eq!(findings, Vec::<Value>::new(), "{}", name(&path));
        let rows: Vec<Value> = report.rows.iter().map(row_json).collect();
        assert_eq!(Value::Array(rows), want["rows"], "{}", name(&path));
        assert_eq!(
            Value::String(tracker::generate(&report.rows).join("\n")),
            want["graph"],
            "{}",
            name(&path)
        );
        // `parse` alone yields the same rows and no malformed line.
        let table = tracker::parse(&body);
        assert_eq!(table.rows, report.rows, "{}", name(&path));
        assert_eq!(table.malformed, Vec::<usize>::new(), "{}", name(&path));
    }
}

#[test]
fn invalid_bodies_report_exactly_their_expected_findings() {
    for path in bodies("invalid") {
        let body = body_of(&path);
        let want = expected(&path);
        let report = tracker::lint(&body);
        let got: Vec<Value> = report.findings.iter().map(finding_json).collect();
        assert_eq!(
            unordered(&got),
            unordered(want["findings"].as_array().expect("findings")),
            "{}",
            name(&path)
        );
        for finding in &report.findings {
            assert!(
                !finding.message().is_empty(),
                "{}: {} has no message",
                name(&path),
                finding.code.as_str()
            );
        }
    }
}

#[test]
fn row_lines_are_one_based_body_lines() {
    let body = body_of(&corpus().join("valid/minimal.md"));
    let lines: Vec<&str> = body.split('\n').collect();
    let report = tracker::lint(&body);
    assert!(!report.rows.is_empty());
    for row in &report.rows {
        assert!(
            lines[row.line - 1].contains(&format!("**{}**", row.id)),
            "row {} is not on line {}",
            row.id,
            row.line
        );
    }
}
