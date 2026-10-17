//! Ownership and read-then-write freshness contracts for existing comment edits.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use pretty_assertions::assert_eq;
use std::fs;
use support::{StubEnv, parse_envelope, run_forge_cli};

const BEFORE: &str = "2026-01-01T00:00:00Z";
const AFTER: &str = "2026-01-01T00:01:00Z";
const STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CALLS"
case "$1:$2" in
 api:user) printf '{"login":"editor"}';;
 api:repos/*/comments/17) case "$*" in
 *PATCH*) printf '{"id":17,"html_url":"https://github.com/example/widget/issues/1#issuecomment-17","body":"Updated comment","updated_at":"2026-01-01T00:01:00Z"}';;
 *) printf '{"id":17,"user":{"login":"%s"},"updated_at":"%s"}' "$AUTHOR" "$UPDATED_AT";;
 esac;;
 *) exit 9;;
esac
"#;

fn fixture(author: &str) -> StubEnv {
    let stub = StubEnv::new().gh_stub(STUB);
    let calls = stub.tempdir.path().join("calls");
    stub.env("CALLS", calls.to_string_lossy())
        .env("AUTHOR", author)
        .env("UPDATED_AT", BEFORE)
}

fn invoke(stub: &StubEnv, extra: &[&str]) -> support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "example/widget",
        "--format",
        "json",
        "comment",
        "edit",
        "17",
        "--body",
        "Updated comment",
    ];
    args.extend_from_slice(extra);
    run_forge_cli(stub, &args)
}

fn calls(stub: &StubEnv) -> String {
    fs::read_to_string(stub.tempdir.path().join("calls")).unwrap_or_default()
}

#[test]
fn own_comment_returns_provider_updated_at_for_both_kinds() {
    for kind in ["issue", "review"] {
        let stub = fixture("editor");
        let result = invoke(&stub, &["--kind", kind]);
        assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
        let data = &parse_envelope(&result.stdout)["data"];
        assert_eq!(data["id"], 17);
        assert_eq!(data["updated_at"], AFTER);
        assert_eq!(data["body"], "Updated comment");
        assert!(data["url"].as_str().unwrap().contains("#issuecomment-17"));
        let log = calls(&stub);
        assert!(log.contains("api user"), "{log}");
        assert_eq!(log.matches("PATCH").count(), 1);
        let category = if kind == "issue" { "issues" } else { "pulls" };
        assert!(log.contains(&format!("repos/example/widget/{category}/comments/17")));
    }
}

#[test]
fn other_author_is_refused_without_a_patch_even_without_expected_timestamp() {
    for kind in ["issue", "review"] {
        let stub = fixture("other-author");
        let result = invoke(&stub, &["--kind", kind]);
        assert_eq!(result.code, 65, "{} {}", result.stdout, result.stderr);
        assert!(result.stdout.contains("comment_author_mismatch"));
        assert!(!calls(&stub).contains("PATCH"));
    }
}

#[test]
fn stale_comment_is_refused_without_a_patch() {
    for kind in ["issue", "review"] {
        let stub = fixture("editor");
        let result = invoke(&stub, &["--kind", kind, "--expected-updated-at", AFTER]);
        assert_eq!(result.code, 65, "{} {}", result.stdout, result.stderr);
        assert!(result.stdout.contains("comment_stale"), "{}", result.stdout);
        assert!(result.stdout.contains("changed"), "{}", result.stdout);
        assert!(!calls(&stub).contains("PATCH"));
    }
}

#[test]
fn matching_timestamp_allows_edit_and_returns_new_timestamp() {
    let stub = fixture("editor");
    let result = invoke(&stub, &["--expected-updated-at", BEFORE]);
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    assert_eq!(parse_envelope(&result.stdout)["data"]["updated_at"], AFTER);
    assert_eq!(calls(&stub).matches("PATCH").count(), 1);
}

#[test]
fn edit_dry_run_is_offline_and_help_discloses_non_atomic_guard() {
    let stub = fixture("other-author");
    let result = invoke(&stub, &["--expected-updated-at", BEFORE, "--dry-run"]);
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    assert!(calls(&stub).is_empty());
    let result = invoke(&stub, &["--help"]);
    assert!(result.stdout.contains("read-then-write"));
    assert!(result.stdout.contains("not atomic"));
}
