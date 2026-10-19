//! Acceptance coverage for inspecting and recovering interrupted own reviews.

use std::fs;

use pretty_assertions::assert_eq;

use super::pr_pending_review::{pending_recovery_script, recovery_snapshot};
use super::pr_review::{github_resumable_thread_stub, run_resumable_thread_review};
use super::support::{StubEnv, parse_envelope, run_forge_cli};

#[test]
fn inspect_reports_creation_time_and_count_before_explicit_discard() {
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("calls.log");
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&recovery_snapshot(false, 2, true)).unwrap();
    snapshot["data"]["node"]["createdAt"] = "2026-07-19T10:00:00Z".into();
    let stub = stub.gh_stub(&pending_recovery_script(
        &capture.to_string_lossy(),
        &snapshot.to_string(),
        "",
    ));
    let inspect = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "pending-review",
            "inspect",
            "42",
            "--review",
            "PRR_pending",
        ],
    );
    assert_eq!(inspect.code, 0, "{}", inspect.stdout);
    let data = &parse_envelope(&inspect.stdout)["data"];
    assert_eq!(data["snapshot"]["review_id"], "PRR_pending");
    assert_eq!(data["snapshot"]["created_at"], "2026-07-19T10:00:00Z");
    assert_eq!(data["inline_comment_count"], 2);
    let digest = data["snapshot"]["snapshot_digest"].as_str().unwrap();
    let discard = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "pending-review",
            "discard",
            "42",
            "--review",
            "PRR_pending",
            "--expected-head",
            "head-new",
            "--expected-commit",
            "head-new",
            "--expected-snapshot",
            digest,
            "--confirm-discard",
            "--confirm-inline-content-loss",
        ],
    );
    assert_eq!(discard.code, 0, "{}", discard.stdout);
    let receipt = parse_envelope(&discard.stdout);
    assert_eq!(receipt["data"]["review_id"], "PRR_pending");
    assert_eq!(receipt["data"]["discarded"], true);
    assert_eq!(receipt["data"]["inline_comment_count"], 2);
}

#[test]
fn interruption_reports_exact_inspect_and_explicit_discard_commands() {
    for failure in ["lost-thread-once", "pending-create-once"] {
        let stub = StubEnv::new();
        let capture = stub.tempdir.path().join("calls.log");
        let thread_file = stub.tempdir.path().join("threads.json");
        fs::write(
            &thread_file,
            r#"[{"path":"src/lib.rs","line":42,"body":"Finding"}]"#,
        )
        .unwrap();
        let stub = stub.gh_stub(&github_resumable_thread_stub(
            &capture.to_string_lossy(),
            false,
            false,
            false,
            failure,
            false,
        ));
        let interrupted = run_resumable_thread_review(&stub, &thread_file);
        assert_eq!(interrupted.code, 65, "{}", interrupted.stdout);
        let envelope = parse_envelope(&interrupted.stdout);
        assert_eq!(
            envelope["error"]["code"],
            "pending_review_transaction_incomplete"
        );
        let detail = envelope["error"]["details"]["detail"].as_str().unwrap();
        assert!(
            detail.contains("pending_review_id=PRR_kwDOpending"),
            "{detail}"
        );
        let fields = detail
            .split("; ")
            .filter_map(|field| field.split_once('='))
            .collect::<std::collections::BTreeMap<_, _>>();
        let prefix =
            "forge-cli --provider github --host 'github.com' --repo 'acme/widgets' --format json";
        let target = "44 --review 'PRR_kwDOpending'";
        let inspect = format!("{prefix} pr pending-review inspect {target}");
        let guards = concat!(
            "--expected-head \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.head_sha')\" ",
            "--expected-commit \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.commit_sha')\" ",
            "--expected-snapshot \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.snapshot_digest')\"",
        );
        let run_id = fields["review_run_id"];
        assert_eq!(fields["inspect_command"], inspect);
        assert_eq!(
            fields["resume_submit_command"],
            format!(
                "snapshot=$({inspect}) && {prefix} pr pending-review resume-submit {target} --review-run-id '{run_id}' --decision comments-only {guards}"
            )
        );
        assert_eq!(
            fields["explicit_discard_command"],
            format!(
                "snapshot=$({inspect}) && {prefix} pr pending-review discard {target} {guards} --confirm-discard --confirm-inline-content-loss"
            )
        );
        let resumed = run_resumable_thread_review(&stub, &thread_file);
        assert_eq!(resumed.code, 0, "{}", resumed.stdout);
        let calls = fs::read_to_string(capture).unwrap();
        assert_eq!(calls.matches("addPullRequestReview(input:").count(), 1);
        assert_eq!(
            calls.matches("addPullRequestReviewThread(input:").count(),
            1
        );
        assert!(!calls.contains("deletePullRequestReview(input:"), "{calls}");
    }
}

#[test]
fn file_level_anchor_mismatch_creates_no_pending_review() {
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("calls.log");
    let thread_file = stub.tempdir.path().join("threads.json");
    fs::write(
        &thread_file,
        r#"[{"path":"src/missing.rs","subject_type":"FILE","body":"Finding"}]"#,
    )
    .unwrap();
    let stub = stub.gh_stub(&github_resumable_thread_stub(
        &capture.to_string_lossy(),
        false,
        false,
        false,
        "none",
        false,
    ));
    let out = run_resumable_thread_review(&stub, &thread_file);
    assert_eq!(out.code, 65, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "review_thread_file_not_changed"
    );
    let calls = fs::read_to_string(capture).unwrap();
    assert!(!calls.contains("mutation("), "{calls}");
    assert!(!calls.contains("--method POST"), "{calls}");
}

#[test]
fn final_snapshot_failure_retains_recovery_identity_and_guidance() {
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("calls.log");
    let thread_file = stub.tempdir.path().join("threads.json");
    fs::write(
        &thread_file,
        r#"[{"path":"src/lib.rs","line":42,"body":"Finding"}]"#,
    )
    .unwrap();
    let script = github_resumable_thread_stub(
        &capture.to_string_lossy(),
        false,
        false,
        false,
        "none",
        false,
    )
    .replace(
        r#"  *"query(\$review: ID!"*)"#,
        r#"  *"query(\$review: ID!"*)
    echo 'snapshot temporarily unavailable' >&2
    exit 42"#,
    );
    let stub = stub.gh_stub(&script);
    for _ in 0..2 {
        let out = run_resumable_thread_review(&stub, &thread_file);
        assert_eq!(out.code, 65, "{}", out.stdout);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(
            envelope["error"]["code"],
            "pending_review_transaction_incomplete"
        );
        let detail = envelope["error"]["details"]["detail"].as_str().unwrap();
        assert!(
            detail.contains("pending_review_id=PRR_kwDOpending"),
            "{detail}"
        );
        assert!(detail.contains("pr pending-review inspect 44"), "{detail}");
    }
    let calls = fs::read_to_string(capture).unwrap();
    assert!(
        calls.contains("addPullRequestReviewThread(input:"),
        "{calls}"
    );
    assert!(!calls.contains("deletePullRequestReview(input:"), "{calls}");
    assert!(!calls.contains("submitPullRequestReview(input:"), "{calls}");
}

#[test]
fn lost_create_keeps_unproven_identity_unknown() {
    for variant in ["foreign", "receipt-mismatch", "unreadable", "head-mismatch"] {
        let stub = StubEnv::new();
        let capture = stub.tempdir.path().join("calls.log");
        let thread_file = stub.tempdir.path().join("threads.json");
        fs::write(
            &thread_file,
            r#"[{"path":"src/lib.rs","line":42,"body":"Finding"}]"#,
        )
        .unwrap();
        let mut script = github_resumable_thread_stub(
            &capture.to_string_lossy(),
            false,
            false,
            false,
            "pending-create-once",
            false,
        );
        let start = script.find(r#"  *"states: [PENDING]"*)"#).unwrap();
        let end = script[start..]
            .find("    ;; ")
            .map(|n| start + n)
            .unwrap_or_else(|| script[start..].find("    ;;\n").unwrap() + start);
        let section = &script[start..end];
        let changed = match variant {
            "foreign" => section.replace("\"viewerDidAuthor\":true", "\"viewerDidAuthor\":false"),
            "head-mismatch" => section.replace(
                r#""headRefOid":"head-44","reviews":{"nodes":[{"#,
                r#""headRefOid":"head-other","reviews":{"nodes":[{"#,
            ),
            _ => section.to_string(),
        };
        script.replace_range(start..end, &changed);
        if variant == "receipt-mismatch" {
            script = script.replace(
                "    body=$(json_escape \"$pending_body\")\n    comments=",
                "    body='Unrelated draft'\n    comments=",
            );
        } else if variant == "unreadable" {
            script = script.replace(
                r#"  *"query(\$review: ID!"*)"#,
                r#"  *"query(\$review: ID!"*)
    echo 'snapshot unavailable' >&2
    exit 42"#,
            );
        }
        let stub = stub.gh_stub(&script);
        let out = run_resumable_thread_review(&stub, &thread_file);
        assert_eq!(out.code, 65, "variant={variant}: {}", out.stdout);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(
            envelope["error"]["code"],
            "pending_review_transaction_incomplete"
        );
        let detail = envelope["error"]["details"]["detail"].as_str().unwrap();
        assert!(
            detail.contains("pending_review_id=<unknown>"),
            "variant={variant}: {detail}"
        );
        assert!(detail.contains("pr reviews 44"), "{detail}");
        assert!(!detail.contains("resume_submit_command="), "{detail}");
        assert!(!detail.contains("explicit_discard_command="), "{detail}");
        let calls = fs::read_to_string(capture).unwrap();
        assert!(!calls.contains("deletePullRequestReview(input:"), "{calls}");
        assert!(!calls.contains("submitPullRequestReview(input:"), "{calls}");
    }
}
