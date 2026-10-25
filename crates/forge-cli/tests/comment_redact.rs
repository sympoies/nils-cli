//! Offline boundary contracts; no real comments are read or mutated.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::fs;
use support::{StubEnv, parse_envelope, run_forge_cli};

const ORIGINAL: &str = "private-fixture-canary";
const REPLACEMENT: &str = "[REDACTED]";
const STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CALLS"
case "$1:$2" in
 api:user) printf '{"login":"maintainer"}';;
 api:repos/example/widget) printf '{"full_name":"example/widget","permissions":{"push":%s}}' "$CAN_WRITE";;
 api:repos/example/widget/issues/comments/17)
 case "$*" in
 *PATCH*)
   test "$MODE" = deny && { printf 'private-fixture-canary' >&2; exit 1; }
   test "$MODE" = lost && { touch "$WRITTEN"; printf 'private-fixture-canary' >&2; exit 1; }
   test "$MODE" = unchanged && exit 1
   touch "$WRITTEN"
   printf 'private-fixture-canary'
   ;;
 *)
   test -f "$WRITTEN" && test "$MODE" = unreadable && { printf 'private-fixture-canary' >&2; exit 1; }
   body='private-fixture-canary'
   test -f "$WRITTEN" && body='[REDACTED]'
   test "$MODE" = already && body='[REDACTED]'
   test "$MODE" = stale && body='Changed fixture'
   test "$MODE" = protected && body='<!-- forge-cli:review-state:v1 fixture -->'
   test "$MODE" = malformed && { printf 'private-fixture-canary'; exit 0; }
   test -f "$WRITTEN" && test "$MODE" = changed && body='Changed fixture'
   body=$(printf '%s' "$body" | base64 | tr -d '\n')
   printf '{"id":%s,"user":{"login":"%s"},"body_base64":"%s","issue_url":"https://api.github.com/repos/%s/issues/1"}' "$COMMENT_ID" "$AUTHOR" "$body" "$COMMENT_REPO"
   ;;
 esac;;
 *) exit 9;;
esac
"#;

fn fixture(mode: &str) -> StubEnv {
    let stub = StubEnv::new().gh_stub(STUB);
    let calls = stub.tempdir.path().join("calls");
    let written = stub.tempdir.path().join("written");
    fs::write(stub.tempdir.path().join("replacement.md"), REPLACEMENT).unwrap();
    stub.env("CALLS", calls.to_string_lossy())
        .env("WRITTEN", written.to_string_lossy())
        .env("CAN_WRITE", "true")
        .env("COMMENT_ID", "17")
        .env("COMMENT_REPO", "example/widget")
        .env("AUTHOR", "retired-bot")
        .env("MODE", mode)
}

fn invoke(stub: &StubEnv, extra: &[&str]) -> support::CmdOutput {
    let digest = Sha256::digest(ORIGINAL)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let path = stub.tempdir.path().join("replacement.md");
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "example/widget",
        "--format",
        "json",
        "comment",
        "redact",
        "17",
        "--expected-author",
        "retired-bot",
        "--expected-body-sha256",
        &digest,
        "--body-file",
        path.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let output = run_forge_cli(stub, &args);
    assert!(!output.stdout.contains(ORIGINAL), "{}", output.stdout);
    assert!(!output.stderr.contains(ORIGINAL), "{}", output.stderr);
    output
}

fn calls(stub: &StubEnv) -> String {
    fs::read_to_string(stub.tempdir.path().join("calls")).unwrap_or_default()
}

#[test]
fn maintainer_redaction_reads_back_and_retry_is_idempotent() {
    let stub = fixture("normal");
    let output = invoke(&stub, &[]);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let data = &parse_envelope(&output.stdout)["data"];
    assert_eq!(data["status"], "verified");
    assert_eq!(data["history_redacted"], false);
    assert!(data.get("body").is_none());
    assert_eq!(calls(&stub).matches("PATCH").count(), 1);
    assert_eq!(invoke(&stub, &[]).code, 0);
    assert_eq!(calls(&stub).matches("PATCH").count(), 1);
}

#[test]
fn lost_write_response_is_recovered_only_by_readback() {
    let stub = fixture("lost");
    let output = invoke(&stub, &[]);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    assert_eq!(parse_envelope(&output.stdout)["data"]["status"], "verified");
    assert_eq!(calls(&stub).matches("PATCH").count(), 1);
}

#[test]
fn unknown_results_never_replay_and_never_emit_backend_content() {
    for mode in ["unchanged", "unreadable", "changed", "deny"] {
        let stub = fixture(mode);
        let output = invoke(&stub, &[]);
        assert_eq!(
            output.code, 69,
            "{mode}: {} {}",
            output.stdout, output.stderr
        );
        let error = &parse_envelope(&output.stdout)["error"];
        assert_eq!(error["code"], "comment_redaction_unresolved");
        assert_eq!(error["details"]["retryable"], false);
        assert_eq!(calls(&stub).matches("PATCH").count(), 1);
    }
}

#[test]
fn stale_author_target_and_permission_refuse_before_write() {
    for (key, value, kind) in [
        ("MODE", "stale", "comment_stale"),
        ("MODE", "protected", "comment_redaction_protected"),
        ("AUTHOR", "unrelated-author", "comment_author_mismatch"),
        ("COMMENT_ID", "18", "comment_response_mismatch"),
        ("COMMENT_REPO", "other/widget", "comment_response_mismatch"),
        ("CAN_WRITE", "false", "comment_redaction_forbidden"),
    ] {
        let stub = fixture("normal").env(key, value);
        let output = invoke(&stub, &[]);
        assert_eq!(
            output.code, 65,
            "{key}: {} {}",
            output.stdout, output.stderr
        );
        assert_eq!(parse_envelope(&output.stdout)["error"]["code"], kind);
        assert!(!calls(&stub).contains("PATCH"));
    }
}

#[test]
fn inspection_exposes_only_target_author_and_digest() {
    let stub = fixture("normal");
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/widget",
            "--format",
            "json",
            "comment",
            "redact",
            "17",
            "--inspect",
        ],
    );
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let data = &parse_envelope(&output.stdout)["data"];
    assert_eq!(data["author"], "retired-bot");
    assert_eq!(
        data["body_sha256"],
        Sha256::digest(ORIGINAL)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert!(data.get("body").is_none());
    assert!(!output.stdout.contains(ORIGINAL));
    assert!(!calls(&stub).contains("PATCH"));
}

#[test]
fn dry_run_is_offline_and_discloses_history_and_concurrency_limits() {
    let stub = fixture("normal");
    let output = invoke(&stub, &["--dry-run"]);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    assert!(calls(&stub).is_empty());
    let output = run_forge_cli(&stub, &["comment", "redact", "--help"]);
    assert!(output.stdout.contains("not atomic"));
    assert!(output.stdout.contains("edit history"));
}

#[test]
fn explicit_repository_and_complete_write_intent_are_required() {
    let stub = fixture("normal");
    let path = stub.tempdir.path().join("replacement.md");
    let output = run_forge_cli(
        &stub,
        &[
            "comment",
            "redact",
            "17",
            "--expected-author",
            "retired-bot",
            "--expected-body-sha256",
            &"a".repeat(64),
            "--body-file",
            path.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "repository_required"
    );
    for extra in [
        vec![],
        vec!["--inspect", "--body-file", path.to_str().unwrap()],
    ] {
        let mut args = vec!["comment", "redact", "17"];
        args.extend(extra);
        assert_eq!(run_forge_cli(&stub, &args).code, 64);
    }
    assert!(calls(&stub).is_empty());
}

#[test]
fn malformed_snapshot_cannot_write_or_leak() {
    let stub = fixture("malformed");
    let output = invoke(&stub, &[]);
    assert_eq!(output.code, 69);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "comment_redaction_read_failed"
    );
    assert!(!calls(&stub).contains("PATCH"));
}

#[test]
fn invalid_digest_stdin_and_empty_replacements_are_offline_refusals() {
    let stub = fixture("normal");
    let path = stub.tempdir.path().join("replacement.md");
    let empty = stub.tempdir.path().join("empty.md");
    fs::write(&empty, "\n").unwrap();
    for (digest, file, kind) in [
        (
            "invalid".to_string(),
            path.to_str().unwrap(),
            "comment_digest_invalid",
        ),
        ("a".repeat(64), "-", "body_file_required"),
        (
            "a".repeat(64),
            empty.to_str().unwrap(),
            "body_missing_summary",
        ),
    ] {
        let output = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "example/widget",
                "--format",
                "json",
                "comment",
                "redact",
                "17",
                "--expected-author",
                "retired-bot",
                "--expected-body-sha256",
                &digest,
                "--body-file",
                file,
            ],
        );
        assert_eq!(output.code, 65, "{} {}", output.stdout, output.stderr);
        assert_eq!(parse_envelope(&output.stdout)["error"]["code"], kind);
    }
    assert!(calls(&stub).is_empty());
}
