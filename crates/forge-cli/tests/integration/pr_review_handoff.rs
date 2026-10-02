//! Assigned review gate fixtures never contact a live provider or session.
use std::fs;

use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::{self, ReviewStatePayload, ReviewStateRecord};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::support::{StubEnv, parse_envelope, run_forge_cli};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn handoff(returned: Option<&str>) -> ReviewHandoff {
    ReviewHandoff {
        coordinator_digest: review_state::sha256_digest(b"worker-session"),
        reviewer_digest: review_state::sha256_digest(b"reviewer-session"),
        review_author: "review-app[bot]".into(),
        base_sha: OLD.into(),
        assigned_head: HEAD.into(),
        returned_reason: returned.map(str::to_string),
    }
}

fn records(h: ReviewHandoff, reviewed_head: Option<&str>) -> Vec<ReviewStateRecord> {
    let first = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewHandoff { handoff: h },
    )
    .unwrap();
    let mut result = vec![first];
    if let Some(head) = reviewed_head {
        let state = review_state::observe_review_loop(None, head, &[])
            .unwrap()
            .state;
        result.push(
            ReviewStateRecord::new(
                "acme/widgets",
                7,
                head,
                1,
                Some(result[0].record_digest.clone()),
                ReviewStatePayload::ReviewLoop { state },
            )
            .unwrap(),
        );
    }
    result
}

fn review(head: &str, verdict: &str) -> Value {
    json!({"id":"REVIEW_1", "databaseId":1,
        "url":"https://github.com/acme/widgets/pull/7#pullrequestreview-1",
        "author":{"login":"review-app[bot]"}, "state":"COMMENTED", "commit":{"oid":head},
        "submittedAt":"2026-07-20T12:00:02Z",
        "body":format!("<!-- agent-kit:specialist-review-report:v1 -->\n## Review Report\n\n- Reviewable: PR #7\n- Lens: testing maintainability\n- Lens verdict: {verdict}\n- Scope: assigned review fixture\n- Evidence reviewed: fixture validation\n\n| Finding | Severity | Confidence | Evidence | Recommendation |\n| --- | --- | ---: | --- | --- |\n| No findings | none | 0.00 | fixture | none |\n"), "viewerDidAuthor":false})
}

fn fixture(head: &str, records: &[ReviewStateRecord], reviews: Vec<Value>) -> StubEnv {
    let stub = StubEnv::new();
    let comments: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "author":{"login":"review-app[bot]"}, "authorAssociation":"OWNER",
                "body":review_state::render_state_comment_body(r, None).unwrap(),
                "createdAt":"2026-07-20T12:00:00Z"
            })
        })
        .collect();
    let ledger = json!({"data":{"viewer":{"login":"review-app[bot]"},
        "repository":{"pullRequest":{"comments":{"nodes":comments,
            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let native = reviews.last().map(|r| {
        json!({"id":r["databaseId"],"html_url":r["url"],
        "state":r["state"],"commit_id":r["commit"]["oid"],"user":r["author"],"body":r["body"]})
    });
    let native = native.unwrap_or(Value::Null);
    let summaries = json!({"data":{"viewer":{"login":"review-app[bot]"},
        "repository":{"pullRequest":{"headRefOid":head,"reviews":{"nodes":reviews,
            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let pending = json!({"data":{"repository":{"pullRequest":{"headRefOid":head,
        "reviews":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let view = json!({"number":7,"url":"https://github.com/acme/widgets/pull/7","state":"OPEN",
        "isDraft":false,"title":"example","headRefName":"feat/example","headRefOid":head,
        "headRepository":{"name":"widgets"},"baseRefName":"main","mergeable":"MERGEABLE",
        "mergedAt":"","mergeCommit":null,"labels":[],"body":"","closingIssuesReferences":[]});
    let log = stub.tempdir.path().join("calls.log");
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> "{log}"
case "$1 $2" in
  "pr view") cat <<'JSON'
{view}
JSON
    exit 0 ;;
  "api repos/acme/widgets/pulls/7/reviews/1") cat <<'JSON'
{native}
JSON
    exit 0 ;;
  "api graphql")
    case "$*" in
      *'states: [PENDING]'*) cat <<'JSON'
{pending}
JSON
        ;;
      *'reviews(first:'*) cat <<'JSON'
{summaries}
JSON
        ;;
      *) cat <<'JSON'
{ledger}
JSON
        ;;
    esac
    exit 0 ;;
esac
echo "unscripted mutation" >&2
exit 99
"#,
        log = log.display()
    );
    stub.gh_stub(&script)
        .env(
            "AGENT_REVIEWER_SESSION",
            "reviewer-session@private-machine-canary",
        )
        .env("AGENT_SESSION_ID", "worker-session")
}

fn check(stub: &StubEnv, head: &str) -> super::support::CmdOutput {
    run_forge_cli(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "check",
            "7",
            "--expected-head",
            head,
        ],
    )
}

fn observe(stub: &StubEnv, head: &str) -> super::support::CmdOutput {
    let findings = stub.tempdir.path().join("clean.json");
    fs::write(&findings, "[]").unwrap();
    run_forge_cli(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            head,
            "--auto-state",
            "--findings-file",
            findings.to_str().unwrap(),
        ],
    )
}

fn assert_refusal(output: &super::support::CmdOutput, code: i32, kind: &str) {
    assert_eq!(output.code, code, "{} {}", output.stdout, output.stderr);
    assert!(output.stdout.contains(kind), "{}", output.stdout);
    assert!(!output.stdout.contains("private-machine-canary"));
}

#[test]
fn assigned_delivery_requires_provider_publication_not_mailbox_pass() {
    let stub =
        fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![]).env("REVIEW_VERDICT", "pass");
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn stale_head_publication_does_not_pass() {
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![review(OLD, "pass")],
    );
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn published_current_head_and_owned_closed_ledger_pass() {
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![review(HEAD, "pass")],
    );
    let output = check(&stub, HEAD);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    assert_eq!(parse_envelope(&output.stdout)["data"]["status"], "reviewed");
    assert!(!output.stdout.contains("private-machine-canary"));
}

#[test]
fn returned_closed_or_unreachable_reviewer_never_enables_self_review() {
    for reason in ["reviewer-closed", "reviewer-unreachable"] {
        let stub = fixture(
            HEAD,
            &records(handoff(Some(reason)), Some(HEAD)),
            vec![review(HEAD, "pass")],
        );
        assert_refusal(&check(&stub, HEAD), 69, "designated_reviewer_unavailable");
        let output = observe(&stub, HEAD);
        assert_eq!(
            parse_envelope(&output.stdout)["data"]["preflight_ok"],
            false
        );
    }
}

#[test]
fn repair_before_first_finding_observation_is_rejected() {
    let stub = fixture(OLD, &records(handoff(None), None), vec![])
        .env("AGENT_SESSION_ID", "reviewer-session");
    let output = observe(&stub, OLD);
    assert_eq!(
        parse_envelope(&output.stdout)["data"]["preflight_ok"],
        false
    );
    assert!(output.stdout.contains("review_repair_unobserved"));
}

#[test]
fn second_writer_cannot_append_even_after_fetching_fresh_tip() {
    for reviewed in [None, Some(HEAD)] {
        let seeded = records(handoff(None), reviewed);
        let reviewer = fixture(HEAD, &seeded, vec![]).env("AGENT_SESSION_ID", "reviewer-session");
        let accepted = observe(&reviewer, HEAD);
        assert_eq!(
            parse_envelope(&accepted.stdout)["data"]["preflight_ok"],
            true,
            "{}",
            accepted.stdout
        );
        let worker = fixture(HEAD, &seeded, vec![]);
        let refused = observe(&worker, HEAD);
        assert_eq!(
            parse_envelope(&refused.stdout)["data"]["preflight_ok"],
            false
        );
        assert!(refused.stdout.contains("review_writer_conflict"));
        assert!(
            !fs::read_to_string(worker.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn newer_blocked_review_cannot_reuse_earlier_pass() {
    let passing = review(HEAD, "pass");
    let mut blocked = review(HEAD, "blocked");
    blocked["databaseId"] = json!(2);
    blocked["submittedAt"] = json!("2026-07-20T12:00:03Z");
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![passing, blocked],
    );
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn prior_self_run_ledger_does_not_satisfy_new_handoff() {
    let state = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let old = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap();
    let assigned = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(old.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: handoff(None),
        },
    )
    .unwrap();
    let stub = fixture(HEAD, &[old, assigned], vec![review(HEAD, "pass")]);
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn merge_preview_cannot_proceed_without_assigned_publication() {
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![]);
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "merge",
            "7",
        ],
    );
    assert_refusal(&output, 65, "awaiting_designated_review");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("pr merge")
    );
}

#[test]
fn worker_cannot_publish_a_competing_native_review() {
    let stub = fixture(HEAD, &records(handoff(None), None), vec![]);
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review",
            "7",
            "--decision",
            "comments-only",
            "--submit-review",
            "--expected-head",
            HEAD,
            "--comment",
            "review evidence",
            "--lens",
            "testing",
        ],
    );
    assert_refusal(&output, 65, "review_writer_conflict");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn handoff_preview_redacts_the_private_selector_and_binds_the_tip() {
    let stub = fixture(HEAD, &[], vec![]);
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "review-handoff",
            "assign",
            "7",
            "--reviewer-session",
            "reviewer-session@private-machine-canary",
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            OLD,
            "--expected-head",
            HEAD,
            "--expected-state",
            "none",
        ],
    );
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let data = parse_envelope(&output.stdout);
    assert_eq!(
        data["data"]["handoff"]["reviewer_digest"],
        review_state::sha256_digest(b"reviewer-session")
    );
    assert!(!output.stdout.contains("private-machine-canary"));
    assert!(!output.stdout.contains("reviewer-session"));
    assert_eq!(data["data"]["status"], "awaiting-designated-review");
}

#[test]
fn handoff_reassignment_requires_the_retained_tip_and_coordinator() {
    let seeded = records(handoff(None), Some(HEAD));
    let tip = seeded.last().unwrap().record_digest.clone();
    for (actor, expected_tip, kind) in [
        ("worker-session", "none", "review_state_conflict"),
        ("another-worker", tip.as_str(), "review_writer_conflict"),
    ] {
        let stub = fixture(HEAD, &seeded, vec![]).env("AGENT_SESSION_ID", actor);
        let output = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "--dry-run",
                "pr",
                "review-handoff",
                "assign",
                "7",
                "--reviewer-session",
                "next-reviewer",
                "--review-author",
                "review-app[bot]",
                "--base-sha",
                OLD,
                "--expected-head",
                HEAD,
                "--expected-state",
                expected_tip,
            ],
        );
        assert_refusal(&output, 65, kind);
    }
}

#[test]
fn coordinator_return_records_a_terminal_unavailable_reason() {
    let seeded = records(handoff(None), None);
    let tip = seeded.last().unwrap().record_digest.clone();
    for reason in ["reviewer-unreachable", "reviewer-closed"] {
        let stub = fixture(HEAD, &seeded, vec![]);
        let output = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "--dry-run",
                "pr",
                "review-handoff",
                "return",
                "7",
                "--expected-head",
                HEAD,
                "--expected-state",
                &tip,
                "--reason",
                reason,
            ],
        );
        assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
        let data = parse_envelope(&output.stdout);
        assert_eq!(data["data"]["status"], "returned-to-coordinator");
        assert_eq!(data["data"]["handoff"]["returned_reason"], reason);
    }
}

#[test]
fn report_before_handoff_or_duplicate_verdict_does_not_pass() {
    for invalid in ["before", "duplicate"] {
        let mut r = review(HEAD, "pass");
        if invalid == "before" {
            r["submittedAt"] = json!("2026-07-20T11:59:59Z");
        } else {
            r["body"] = json!(format!(
                "{}\n- Lens verdict: blocked",
                r["body"].as_str().unwrap()
            ));
        }
        let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![r]);
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    }
}

#[test]
fn long_published_report_uses_bounded_native_body_readback() {
    let mut r = review(HEAD, "pass");
    r["body"] = json!(format!(
        "{}\n{}",
        r["body"].as_str().unwrap(),
        "evidence ".repeat(600)
    ));
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![r]);
    let output = check(&stub, HEAD);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    assert!(
        fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("repos/acme/widgets/pulls/7/reviews/1")
    );
}
