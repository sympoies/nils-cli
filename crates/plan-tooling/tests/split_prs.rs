use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("split_prs")
        .join(name)
}

#[test]
fn split_prs_fixture_tsv_header_is_stable() {
    for file in ["per_sprint_expected.tsv", "group_expected.tsv"] {
        let path = fixture_path(file);
        let text = fs::read_to_string(path).expect("fixture exists");
        let first = text.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            "# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group"
        );
    }
}

#[test]
fn split_prs_fixture_json_contains_required_fields() {
    for file in ["per_sprint_expected.json", "group_expected.json"] {
        let path = fixture_path(file);
        let text = fs::read_to_string(path).expect("fixture exists");
        let value: Value = serde_json::from_str(&text).expect("valid json");

        assert!(value["file"].is_string());
        assert!(value["scope"].is_string());
        assert!(value["pr_grouping"].is_string());
        assert!(value["strategy"].is_string());
        let records = value["records"].as_array().expect("records array");
        assert!(!records.is_empty());

        for record in records {
            assert!(record["task_id"].is_string());
            assert!(record["summary"].is_string());
            assert!(record["branch"].is_string());
            assert!(record["worktree"].is_string());
            assert!(record["owner"].is_string());
            assert!(record["notes"].is_string());
            assert!(record["pr_group"].is_string());
        }
    }
}

#[test]
fn split_prs_error_matrix_doc_mentions_core_cases() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("specs")
        .join("split-prs-contract-v1.md");
    let text = fs::read_to_string(path).expect("spec exists");

    for token in [
        "unknown mapping key",
        "missing mapping",
        "--pr-grouping",
        "scope=sprint",
    ] {
        assert!(
            text.contains(token),
            "expected token in error matrix: {token}"
        );
    }
}
