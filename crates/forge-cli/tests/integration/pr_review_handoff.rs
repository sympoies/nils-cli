//! Assigned review gate fixtures never contact a live provider or session.
use std::fs;

use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::{self, ReviewStatePayload, ReviewStateRecord};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::support::{StubEnv, parse_envelope, run_forge_cli};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Full canonical session UUIDs for the ownership selectors (`--reviewer-session`,
/// `--coordinator-session`), which must name the real full-UUID identity. A short
/// prefix of the reviewer UUID is used to exercise the non-canonical rejection.
const COORDINATOR_UUID: &str = "123e4567-e89b-42d3-a456-426614174000";
const REVIEWER_UUID: &str = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";
const OTHER_COORDINATOR_UUID: &str = "00112233-4455-6677-8899-aabbccddeeff";

/// A handoff whose coordinator is a full canonical UUID, as the ownership
/// selectors require. Used by the coordinator-retired takeover fixtures, where
/// `--coordinator-session` must digest to the recorded coordinator identity.
fn retired_coordinator_handoff() -> ReviewHandoff {
    let mut h = handoff(None);
    h.coordinator_digest = review_state::sha256_digest(COORDINATOR_UUID.as_bytes());
    h
}

fn handoff(returned: Option<&str>) -> ReviewHandoff {
    ReviewHandoff {
        coordinator_digest: review_state::sha256_digest(b"worker-session"),
        reviewer_digest: review_state::sha256_digest(b"reviewer-session"),
        review_author: "review-app[bot]".into(),
        base_sha: OLD.into(),
        assigned_head: HEAD.into(),
        returned_reason: returned.map(str::to_string),
        assignment_generation: if returned.is_some() { 2 } else { 1 },
        surrendered: false,
        coordinator_transfer: None,
    }
}

fn records(h: ReviewHandoff, reviewed_head: Option<&str>) -> Vec<ReviewStateRecord> {
    let terminal = h.returned_reason.is_some();
    let mut initial = h.clone();
    if terminal {
        initial.returned_reason = None;
        initial.assignment_generation = 1;
    }
    let owner_generation = initial.assignment_generation;
    let first = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewHandoff { handoff: initial },
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
                result.len() as u64,
                Some(result.last().unwrap().record_digest.clone()),
                ReviewStatePayload::ReviewLoop { state },
            )
            .unwrap()
            .with_assignment_generation(Some(owner_generation))
            .unwrap(),
        );
    }
    if terminal {
        result.push(
            ReviewStateRecord::new(
                "acme/widgets",
                7,
                HEAD,
                result.len() as u64,
                Some(result.last().unwrap().record_digest.clone()),
                ReviewStatePayload::ReviewHandoff { handoff: h },
            )
            .unwrap(),
        );
    }
    result
}

pub(super) fn review(head: &str, verdict: &str) -> Value {
    json!({"id":"REVIEW_1", "databaseId":1,
        "url":"https://github.com/acme/widgets/pull/7#pullrequestreview-1",
        "author":{"login":"review-app[bot]", "__typename":"Bot", "id":"BOT_REVIEW_APP"}, "state":"COMMENTED", "commit":{"oid":head},
        "submittedAt":"2026-07-20T12:00:02Z",
        "body":format!("<!-- agent-kit:specialist-review-report:v1 -->\n## Review Report\n\n- Reviewable: PR #7\n- Lens: testing maintainability\n- Lens verdict: {verdict}\n- Scope: assigned review fixture\n- Evidence reviewed: fixture validation\n\n| Finding | Severity | Confidence | Evidence | Recommendation |\n| --- | --- | ---: | --- | --- |\n| No findings | none | 0.00 | fixture | none |\n"), "viewerDidAuthor":false})
}

pub(super) fn fixture(head: &str, records: &[ReviewStateRecord], reviews: Vec<Value>) -> StubEnv {
    fixture_with_native(head, records, reviews, None)
}

fn fixture_with_native(
    head: &str,
    records: &[ReviewStateRecord],
    reviews: Vec<Value>,
    native_override: Option<Value>,
) -> StubEnv {
    fixture_with_native_and_comments(head, records, reviews, native_override, None)
}

fn fixture_with_native_and_comments(
    head: &str,
    records: &[ReviewStateRecord],
    reviews: Vec<Value>,
    native_override: Option<Value>,
    comments_override: Option<Value>,
) -> StubEnv {
    let stub = StubEnv::new();
    let comments: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "author":{"login":"review-app[bot]"}, "authorAssociation":"OWNER",
                "body":review_state::render_state_comment_body(r, None).unwrap(),
                "createdAt": if matches!(r.payload, ReviewStatePayload::ReviewHandoff { .. }) {
                    "2026-07-20T12:00:00Z"
                } else {
                    "2026-07-20T12:00:03Z"
                }
            })
        })
        .collect();
    let ledger = json!({"data":{"viewer":{"login":"review-app[bot]"},
        "repository":{"pullRequest":{"comments":{"nodes":comments,
            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let native_cases = reviews
        .iter()
        .map(|r| {
            let native = native_override.clone().unwrap_or_else(|| {
                json!({"id":r["databaseId"],"html_url":r["url"],
                "state":r["state"],"commit_id":r["commit"]["oid"],
                "user":{"login":r["author"]["login"], "type":r["author"]["__typename"], "node_id":r["author"]["id"]},"body":r["body"]})
            });
            let id = r["databaseId"].as_u64().unwrap();
            let comments = comments_override.clone().unwrap_or_else(|| {
                json!([{"pull_request_review_id":id, "commit_id":head, "in_reply_to_id":9}])
            });
            format!(
                "  \"api repos/acme/widgets/pulls/7/reviews/{id}\") cat <<'JSON'\n{native}\nJSON\n    exit 0 ;;\n  \"api repos/acme/widgets/pulls/7/reviews/{id}/comments?per_page=100&page=1\") cat <<'JSON'\n{comments}\nJSON\n    exit 0 ;;"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
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
  "api repos/acme/widgets/pulls/7") echo "${{PROVIDER_BASE:-{OLD}}}"; exit 0 ;;
{native_cases}
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
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1")
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

pub(super) fn assert_refusal(output: &super::support::CmdOutput, code: i32, kind: &str) {
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
fn thread_reply_review_empty_comments_preserve_published_approval() {
    // GitHub creates an empty COMMENTED review for both a REST thread reply
    // and the reply posted by resolve --note. Neither replaces the report.
    for actor in ["reviewer-user", "review-app[bot]"] {
        for replies in [1, 2] {
            let mut owner = handoff(None);
            owner.review_author = actor.into();
            let mut approved = review(HEAD, "pass");
            approved["state"] = json!("APPROVED");
            if actor == "reviewer-user" {
                approved["author"] = json!({
                    "login": actor, "__typename": "User", "id": "USER_REVIEWER"
                });
            }
            let mut reviews = vec![approved.clone()];
            for index in 0..replies {
                let mut reply = approved.clone();
                reply["id"] = json!(format!("REVIEW_REPLY_{index}"));
                reply["databaseId"] = json!(index + 2);
                reply["url"] = json!(format!(
                    "https://github.com/acme/widgets/pull/7#pullrequestreview-{}",
                    index + 2
                ));
                reply["state"] = json!("COMMENTED");
                reply["body"] = json!("");
                reply["submittedAt"] = json!(format!("2026-07-20T12:00:0{}Z", index + 3));
                reviews.push(reply);
            }
            let stub = fixture(HEAD, &records(owner, Some(HEAD)), reviews);
            let output = check(&stub, HEAD);
            assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
            assert_eq!(parse_envelope(&output.stdout)["data"]["status"], "reviewed");
        }
    }
}

#[test]
fn thread_reply_review_substantive_comment_still_rejects_earlier_pass() {
    let mut approved = review(HEAD, "pass");
    approved["state"] = json!("APPROVED");
    let mut comment = review(HEAD, "pass");
    comment["databaseId"] = json!(2);
    comment["body"] = json!("A substantive review without a canonical report.");
    comment["submittedAt"] = json!("2026-07-20T12:00:03Z");
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![approved, comment],
    );
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn thread_reply_review_empty_other_states_cannot_reuse_earlier_pass() {
    for state in ["APPROVED", "CHANGES_REQUESTED", "DISMISSED"] {
        let mut approved = review(HEAD, "pass");
        approved["state"] = json!("APPROVED");
        let mut newer = review(HEAD, "pass");
        newer["databaseId"] = json!(2);
        newer["state"] = json!(state);
        newer["body"] = json!("");
        newer["submittedAt"] = json!("2026-07-20T12:00:03Z");
        let stub = fixture(
            HEAD,
            &records(handoff(None), Some(HEAD)),
            vec![approved, newer],
        );
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    }
}

#[test]
fn thread_reply_review_inline_comments_and_incomplete_provenance_fail_closed() {
    let mut approved = review(HEAD, "pass");
    approved["state"] = json!("APPROVED");
    let mut empty = review(HEAD, "pass");
    empty["databaseId"] = json!(2);
    empty["body"] = json!("");
    empty["submittedAt"] = json!("2026-07-20T12:00:03Z");
    for (comments, error) in [
        (
            json!([{"pull_request_review_id":2,"commit_id":HEAD}]),
            "awaiting_designated_review",
        ),
        (json!([{"in_reply_to_id":9}]), "review_snapshot_incomplete"),
        (
            json!([{"pull_request_review_id":3,"commit_id":HEAD,"in_reply_to_id":9}]),
            "review_snapshot_incomplete",
        ),
        (
            json!({"message":"incomplete"}),
            "review_snapshot_incomplete",
        ),
        (
            json!(vec![
                json!({"pull_request_review_id":2,"commit_id":HEAD,"in_reply_to_id":9});
                100
            ]),
            "review_snapshot_incomplete",
        ),
    ] {
        let stub = fixture_with_native_and_comments(
            HEAD,
            &records(handoff(None), Some(HEAD)),
            vec![approved.clone(), empty.clone()],
            None,
            Some(comments),
        );
        assert_refusal(&check(&stub, HEAD), 65, error);
    }
}

#[test]
fn thread_reply_review_incomplete_native_body_or_identity_fails_closed() {
    let mut approved = review(HEAD, "pass");
    approved["state"] = json!("APPROVED");
    let mut empty = review(HEAD, "pass");
    empty["databaseId"] = json!(2);
    empty["body"] = json!("");
    empty["submittedAt"] = json!("2026-07-20T12:00:03Z");
    for field in ["body", "author", "head"] {
        let mut native = json!({
            "id":2, "html_url":empty["url"], "state":"COMMENTED", "commit_id":HEAD,
            "user":{"login":"review-app[bot]","type":"Bot","node_id":"BOT_REVIEW_APP"},
            "body":""
        });
        match field {
            "body" => native["body"] = Value::Null,
            "author" => native["user"]["node_id"] = json!("OTHER_BOT"),
            "head" => native["commit_id"] = json!(OLD),
            _ => unreachable!(),
        }
        let stub = fixture_with_native(
            HEAD,
            &records(handoff(None), Some(HEAD)),
            vec![approved.clone(), empty.clone()],
            Some(native),
        );
        assert_refusal(&check(&stub, HEAD), 65, "review_snapshot_incomplete");
    }
}

#[test]
fn thread_reply_review_other_author_or_head_cannot_satisfy_admission() {
    for field in ["author", "head"] {
        let mut empty = review(HEAD, "pass");
        empty["body"] = json!("");
        if field == "author" {
            empty["author"]["login"] = json!("other-review-app[bot]");
        } else {
            empty["commit"]["oid"] = json!(OLD);
        }
        let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![empty]);
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    }
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
            &format!("{REVIEWER_UUID}@private-machine-canary"),
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
        review_state::sha256_digest(REVIEWER_UUID.as_bytes())
    );
    assert!(!output.stdout.contains("private-machine-canary"));
    assert!(!output.stdout.contains(REVIEWER_UUID));
    assert_eq!(data["data"]["status"], "awaiting-designated-review");
}

#[test]
fn handoff_assign_rejects_noncanonical_reviewer_session() {
    // A short prefix must be rejected: it would digest to an identity the
    // reviewer's real full-UUID identity never matches, so the reviewer would
    // later fail with a writer conflict. Prefixes are not resolved.
    let stub = fixture(HEAD, &[], vec![]).env("AGENT_SESSION_ID", COORDINATOR_UUID);
    let prefix = run_forge_cli(
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
            &format!("{}@private-machine-canary", &REVIEWER_UUID[..8]),
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
    assert_eq!(prefix.code, 65, "{} {}", prefix.stdout, prefix.stderr);
    assert!(
        prefix.stdout.contains("the full session UUID is required"),
        "{}",
        prefix.stdout
    );
    assert!(!prefix.stdout.contains("private-machine-canary"));

    // The full canonical UUID is accepted and digests as the reviewer's real identity.
    let full = run_forge_cli(
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
            &format!("{REVIEWER_UUID}@private-machine-canary"),
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
    assert_eq!(full.code, 0, "{} {}", full.stdout, full.stderr);
    let data = parse_envelope(&full.stdout);
    assert_eq!(
        data["data"]["handoff"]["reviewer_digest"],
        review_state::sha256_digest(REVIEWER_UUID.as_bytes())
    );
    assert!(!full.stdout.contains("private-machine-canary"));
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
fn coordinator_recovery_records_a_terminal_unavailable_reason() {
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
                "recover",
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

fn app_review_pair() -> (Value, Value) {
    let mut graphql = review(HEAD, "pass");
    graphql["state"] = json!("APPROVED");
    graphql["author"] = json!({"login":"review-app", "__typename":"Bot", "id":"BOT_REVIEW_APP"});
    let rest = json!({"id":graphql["databaseId"], "html_url":graphql["url"],
        "state":graphql["state"], "commit_id":HEAD, "body":graphql["body"],
        "user":{"login":"review-app[bot]", "type":"Bot", "node_id":"BOT_REVIEW_APP", "id":101}});
    (graphql, rest)
}

#[test]
fn app_identity_rest_and_graphql_forms_admit_the_same_review() {
    for long_body in [false, true] {
        for exact_login in [false, true] {
            let (mut graphql, mut rest) = app_review_pair();
            if exact_login {
                graphql["author"]["login"] = json!("review-app[bot]");
            }
            if long_body {
                rest["body"] = json!(format!(
                    "{}\n{}",
                    rest["body"].as_str().unwrap(),
                    "evidence ".repeat(600)
                ));
                graphql["body"] = rest["body"].clone();
            }
            let stub = fixture_with_native(
                HEAD,
                &records(handoff(None), Some(HEAD)),
                vec![graphql],
                Some(rest),
            );
            let output = check(&stub, HEAD);
            assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
            assert_eq!(parse_envelope(&output.stdout)["data"]["status"], "reviewed");
            let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
            assert!(
                calls.contains("author { login __typename ... on Node { id } }"),
                "{calls}"
            );
        }
    }
}

#[test]
fn app_identity_distinct_or_ambiguous_authors_are_rejected() {
    for invalid in [
        "exact-missing-type",
        "exact-missing-node",
        "exact-user",
        "different-bot",
        "same-name-user",
        "missing-type",
        "missing-node",
        "bare-expected",
    ] {
        let (mut graphql, rest) = app_review_pair();
        let mut h = handoff(None);
        match invalid {
            "exact-missing-type" => {
                graphql["author"] = json!({"login":"review-app[bot]", "id":"BOT_REVIEW_APP"})
            }
            "exact-missing-node" => {
                graphql["author"] = json!({"login":"review-app[bot]", "__typename":"Bot"})
            }
            "exact-user" => {
                graphql["author"] =
                    json!({"login":"review-app[bot]", "__typename":"User", "id":"USER_REVIEW_APP"})
            }
            "different-bot" => {
                graphql["author"] =
                    json!({"login":"another-app", "__typename":"Bot", "id":"BOT_OTHER"})
            }
            "same-name-user" => {
                graphql["author"] =
                    json!({"login":"review-app", "__typename":"User", "id":"USER_REVIEW_APP"})
            }
            "missing-type" => {
                graphql["author"]
                    .as_object_mut()
                    .unwrap()
                    .remove("__typename");
            }
            "missing-node" => {
                graphql["author"].as_object_mut().unwrap().remove("id");
            }
            "bare-expected" => h.review_author = "review-app".into(),
            _ => unreachable!(),
        }
        let stub = fixture_with_native(HEAD, &records(h, Some(HEAD)), vec![graphql], Some(rest));
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    }
}

#[test]
fn app_identity_readback_must_bind_the_bot_and_review() {
    for invalid in ["node", "missing-node", "type", "login", "review-id", "head"] {
        for exact_login in [false, true] {
            let (mut graphql, mut rest) = app_review_pair();
            if exact_login {
                graphql["author"]["login"] = json!("review-app[bot]");
            }
            match invalid {
                "node" => rest["user"]["node_id"] = json!("BOT_OTHER"),
                "missing-node" => {
                    rest["user"].as_object_mut().unwrap().remove("node_id");
                }
                "type" => rest["user"]["type"] = json!("User"),
                "login" => rest["user"]["login"] = json!("another-app[bot]"),
                "review-id" => rest["id"] = json!(2),
                "head" => rest["commit_id"] = json!(OLD),
                _ => unreachable!(),
            }
            let stub = fixture_with_native(
                HEAD,
                &records(handoff(None), Some(HEAD)),
                vec![graphql],
                Some(rest),
            );
            assert_refusal(&check(&stub, HEAD), 65, "review_snapshot_incomplete");
        }
    }
}

#[test]
fn restarted_worker_cannot_publish_without_assignment_environment() {
    let stub =
        fixture(HEAD, &records(handoff(None), None), vec![]).env("AGENT_REVIEWER_SESSION", "");
    let out = run_forge_cli(
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
    assert_refusal(&out, 65, "review_writer_conflict");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn mixed_case_designated_author_is_the_same_github_identity() {
    let mut h = handoff(None);
    h.review_author = "Review-App[bot]".into();
    let stub = fixture(HEAD, &records(h, Some(HEAD)), vec![review(HEAD, "pass")]);
    let out = check(&stub, HEAD);
    assert_eq!(out.code, 0, "{}", out.stdout);
}

#[test]
fn canonical_report_for_another_pr_does_not_satisfy_handoff() {
    let mut r = review(HEAD, "pass");
    r["body"] = json!(r["body"].as_str().unwrap().replace("PR #7", "PR #8"));
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![r]);
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn first_owned_same_head_checkpoint_preserves_and_appends_inherited_state() {
    let inherited = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let first = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewLoop { state: inherited },
    )
    .unwrap();
    let assignment = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(first.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: handoff(None),
        },
    )
    .unwrap();
    let stub =
        fixture(HEAD, &[first, assignment], vec![]).env("AGENT_SESSION_ID", "reviewer-session");
    let out = observe(&stub, HEAD);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["preflight_ok"], true, "{}", out.stdout);
    assert_eq!(data["data"]["would_append"], true, "{}", out.stdout);
}

#[test]
fn coordinator_recovery_wins_over_a_racing_old_writer_child() {
    let root = records(handoff(None), None).remove(0);
    let old_state = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state: old_state },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    let recovery = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: handoff(Some("reviewer-closed")),
        },
    )
    .unwrap();
    for race in [
        vec![root.clone(), observation.clone(), recovery.clone()],
        vec![root.clone(), recovery.clone(), observation.clone()],
    ] {
        let comments: Vec<String> = race.iter().map(|r| r.marker().unwrap()).collect();
        let chain =
            review_state::parse_chain(comments.iter().map(String::as_str), "acme/widgets", 7)
                .expect("explicit newer recovery must preserve a parseable winning chain");
        assert_eq!(chain.tip_digest, Some(recovery.record_digest.clone()));
        assert_eq!(chain.records.len(), 2);
    }
}

#[test]
fn scope_refresh_wins_over_a_racing_old_writer_and_refuses_competing_refreshes() {
    let root = records(handoff(None), None).remove(0);
    let state = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let old_write = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    let mut next = handoff(None);
    next.base_sha = HEAD.into();
    next.assignment_generation = 2;
    let refresh = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: next.clone(),
        },
    )
    .unwrap();
    for children in [
        vec![old_write.clone(), refresh.clone()],
        vec![refresh.clone(), old_write.clone()],
    ] {
        let mut markers = vec![root.marker().unwrap()];
        markers.extend(children.iter().map(|r| r.marker().unwrap()));
        let chain =
            review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7)
                .expect("a fresh reviewer interval fences the racing old-generation write");
        assert_eq!(chain.tip_digest, Some(refresh.record_digest.clone()));
        assert_eq!(
            chain.ignored_stale_records,
            vec![old_write.record_digest.clone()]
        );
    }
    next.assigned_head = OLD.into();
    let competing = ReviewStateRecord::new(
        "acme/widgets",
        7,
        OLD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff { handoff: next },
    )
    .unwrap();
    let markers = [
        root.marker().unwrap(),
        refresh.marker().unwrap(),
        competing.marker().unwrap(),
    ];
    assert!(
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).is_err()
    );
}

#[test]
fn surrender_and_explicit_recovery_have_distinct_role_fences() {
    let seeded = records(handoff(None), None);
    for (command, actor, reason) in [
        ("surrender", "reviewer-session", false),
        ("recover", "worker-session", true),
    ] {
        let stub = fixture(HEAD, &seeded, vec![]).env("AGENT_SESSION_ID", actor);
        let tip = seeded.last().unwrap().record_digest.as_str();
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "review-handoff",
            command,
            "7",
            "--expected-head",
            HEAD,
            "--expected-state",
            tip,
        ];
        if reason {
            args.extend(["--reason", "reviewer-closed"]);
        }
        let out = run_forge_cli(&stub, &args);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    }
}

#[test]
fn handoff_operation_uses_the_existing_catalog_schema() {
    let catalog: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(include_str!("../../docs/specs/forge-cli-ops-v1.yaml")).unwrap();
    let operation = catalog["operations"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|op| op["id"].as_str() == Some("pr.review-handoff"))
        .unwrap();
    assert!(
        operation.get("output").is_some(),
        "operation must expose canonical output"
    );
    assert!(operation.get("outputs").is_none());
    assert!(
        catalog["validations_catalog"]["designated_review_handoff"]["error_kind"]
            .as_str()
            .unwrap()
            .split(" | ")
            .any(|kind| kind == "review_assignment_missing")
    );
    assert!(
        !operation["backends"]["github"]["note"]
            .as_str()
            .unwrap()
            .contains("assign/return")
    );
    for rule in operation["validations"].as_sequence().unwrap() {
        assert!(
            catalog["validations_catalog"]
                .get(rule.as_str().unwrap())
                .is_some(),
            "unknown catalog validation {rule:?}"
        );
    }
}

#[test]
fn provider_base_drift_and_stale_assignment_generation_are_rejected() {
    let seeded = records(handoff(None), Some(HEAD));
    let moved = fixture(HEAD, &seeded, vec![review(HEAD, "pass")]).env("PROVIDER_BASE", HEAD);
    assert_refusal(&check(&moved, HEAD), 65, "review_scope_changed");
    let stale = fixture(HEAD, &seeded, vec![])
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "0");
    let out = observe(&stale, HEAD);
    assert_eq!(parse_envelope(&out.stdout)["data"]["preflight_ok"], false);
    assert!(out.stdout.contains("review_assignment_generation_conflict"));
}

#[test]
fn handover_preserves_unresolved_findings_and_history() {
    let observation = review_state::ReviewFindingObservation {
        fingerprint: "correctness:fixture:retained-finding".into(),
        root_cause_fingerprint: None,
        blocking: true,
        status: review_state::ReviewFindingStatus::Open,
        threads: vec![],
    };
    let inherited =
        review_state::observe_review_loop(None, HEAD, std::slice::from_ref(&observation))
            .unwrap()
            .state;
    let record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewLoop {
            state: inherited.clone(),
        },
    )
    .unwrap();
    let assigned = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(record.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: handoff(None),
        },
    )
    .unwrap();
    let comments = [record.marker().unwrap(), assigned.marker().unwrap()];
    let chain =
        review_state::parse_chain(comments.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    let previous = review_state::latest_review_loop_state(&chain).unwrap();
    assert_eq!(previous, &inherited);
    let transition =
        review_state::observe_review_loop(Some(previous), HEAD, std::slice::from_ref(&observation))
            .unwrap();
    assert_eq!(transition.state, inherited);
    let stub =
        fixture(HEAD, &[record, assigned], vec![]).env("AGENT_SESSION_ID", "reviewer-session");
    let findings = stub.tempdir.path().join("retained.json");
    fs::write(&findings, serde_json::to_vec(&vec![observation]).unwrap()).unwrap();
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
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            HEAD,
            "--auto-state",
            "--findings-file",
            findings.to_str().unwrap(),
        ],
    );
    assert_eq!(
        parse_envelope(&output.stdout)["data"]["would_append"],
        true,
        "{}",
        output.stdout
    );
}

#[test]
fn concurrent_recovery_and_observation_recheck_the_tip_before_either_post() {
    for recovery_first in [true, false] {
        let root = records(handoff(None), None).remove(0);
        let winner = if recovery_first {
            ReviewStateRecord::new(
                "acme/widgets",
                7,
                HEAD,
                1,
                Some(root.record_digest.clone()),
                ReviewStatePayload::ReviewHandoff {
                    handoff: handoff(Some("reviewer-closed")),
                },
            )
            .unwrap()
        } else {
            let state = review_state::observe_review_loop(None, HEAD, &[])
                .unwrap()
                .state;
            ReviewStateRecord::new(
                "acme/widgets",
                7,
                HEAD,
                1,
                Some(root.record_digest.clone()),
                ReviewStatePayload::ReviewLoop { state },
            )
            .unwrap()
            .with_assignment_generation(Some(1))
            .unwrap()
        };
        let ledger = |records: &[ReviewStateRecord]| {
            let nodes: Vec<_> = records.iter().map(|r|json!({"author":{"login":"review-app[bot]"},
                "authorAssociation":"OWNER","createdAt":"2026-07-20T12:00:00Z","body":r.marker().unwrap()})).collect();
            json!({"data":{"viewer":{"login":"review-app[bot]"},"repository":{"pullRequest":{
                "comments":{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}})
        };
        let before = ledger(std::slice::from_ref(&root));
        let after = ledger(&[root.clone(), winner]);
        let stub = fixture(HEAD, &[], vec![]).env(
            "AGENT_SESSION_ID",
            if recovery_first {
                "reviewer-session"
            } else {
                "worker-session"
            },
        );
        let inner = stub.tempdir.path().join("gh-inner");
        fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
        let counter = stub.tempdir.path().join("state-read");
        let wrapper = format!(
            r#"#!/bin/sh
case "$*" in *"comments(first: 100, after:"*)
  if [ -e '{counter}' ]; then
    cat <<'JSON'
{after}
JSON
  else
    touch '{counter}'
    cat <<'JSON'
{before}
JSON
  fi
  exit 0 ;;
esac
exec '{inner}' "$@"
"#,
            counter = counter.display(),
            inner = inner.display()
        );
        let stub = stub.gh_stub(&wrapper);
        let findings = stub.tempdir.path().join("empty.json");
        fs::write(&findings, "[]").unwrap();
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
        ];
        if recovery_first {
            args.extend([
                "review-loop",
                "observe",
                "7",
                "--findings-file",
                findings.to_str().unwrap(),
            ]);
        } else {
            args.extend([
                "review-handoff",
                "recover",
                "7",
                "--reason",
                "reviewer-closed",
            ]);
        }
        args.extend([
            "--expected-head",
            HEAD,
            "--expected-state",
            &root.record_digest,
        ]);
        let out = run_forge_cli(&stub, &args);
        assert_refusal(&out, 65, "review_state_conflict");
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn live_same_head_handoff_appends_reconciles_and_admits_published_review() {
    let inherited = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let inherited_record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewLoop {
            state: inherited.clone(),
        },
    )
    .unwrap();
    let assigned = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(inherited_record.record_digest.clone()),
        ReviewStatePayload::ReviewHandoff {
            handoff: handoff(None),
        },
    )
    .unwrap();
    let stub = fixture(
        HEAD,
        &[inherited_record.clone(), assigned.clone()],
        vec![review(HEAD, "pass")],
    )
    .env("AGENT_SESSION_ID", "reviewer-session");
    let gh = stub.tempdir.path().join("gh");
    let inner = stub.tempdir.path().join("inner-gh");
    fs::rename(&gh, &inner).unwrap();
    let posted = stub.tempdir.path().join("posted-body");
    let wrapper = format!(
        r#"#!/usr/bin/env python3
import json, subprocess, sys
from pathlib import Path
args = sys.argv[1:]
posted = Path({posted:?})
if args[:2] == ['api', 'repos/acme/widgets/issues/7/comments']:
    body = next(a[len('body='):] for a in args if a.startswith('body='))
    posted.write_text(body)
    print(json.dumps({{'html_url':'https://github.com/acme/widgets/pull/7#issuecomment-2'}}))
    sys.exit(0)
r = subprocess.run([{inner:?}] + args, capture_output=True, text=True)
if r.returncode == 0 and args[:2] == ['api', 'graphql'] and 'comments(first:' in ' '.join(args) and posted.exists():
    data = json.loads(r.stdout)
    data['data']['repository']['pullRequest']['comments']['nodes'].append({{
        'author':{{'login':'review-app[bot]'}}, 'authorAssociation':'OWNER',
        'createdAt':'2026-07-20T12:00:01Z', 'body':posted.read_text()}})
    print(json.dumps(data))
else:
    sys.stdout.write(r.stdout)
sys.stderr.write(r.stderr)
sys.exit(r.returncode)
"#,
        posted = posted.to_str().unwrap(),
        inner = inner.to_str().unwrap()
    );
    fs::write(&gh, wrapper).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let findings = stub.tempdir.path().join("clean-live.json");
    fs::write(&findings, "[]").unwrap();
    let args = [
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "pr",
        "review-loop",
        "observe",
        "7",
        "--expected-head",
        HEAD,
        "--auto-state",
        "--findings-file",
        findings.to_str().unwrap(),
    ];
    let output = run_forge_cli(&stub, &args);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    assert_eq!(parse_envelope(&output.stdout)["data"]["appended"], true);
    let body = fs::read_to_string(&posted).unwrap();
    let bodies = [
        inherited_record.marker().unwrap(),
        assigned.marker().unwrap(),
        body,
    ];
    let chain =
        review_state::parse_chain(bodies.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), 3);
    assert_eq!(chain.records.last().unwrap().assignment_generation, Some(1));
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&inherited)
    );
    let admission = check(&stub, HEAD);
    assert_eq!(
        admission.code, 0,
        "{} {}",
        admission.stdout, admission.stderr
    );
    let retry = run_forge_cli(&stub, &args);
    assert_eq!(retry.code, 0, "{} {}", retry.stdout, retry.stderr);
    assert_eq!(parse_envelope(&retry.stdout)["data"]["appended"], false);
}

fn coordinator_board(stub: StubEnv, state: &str, reason: &str) -> StubEnv {
    let now = jiff::Timestamp::now().to_string();
    let board = json!({"ok":true,"data":{"mode":"local","board":{
        "schema_version":"agent-session.board-view.v1", "generated_at":now,
        "truncated":false, "machines":[{"machine":"source", "available":true,"last_seen_at":now}],
        "records":[{"session_id":COORDINATOR_UUID, "machine":"source", "state":state,
            "runtime_status":if state == "live" { json!("running") } else { Value::Null },
            "closed_at":if state == "closed" { json!(now) } else { Value::Null },
            "close_reason":reason}]
    }}});
    let path = stub.write_stub(
        "agent-session",
        &format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
    );
    stub.env("FORGE_CLI_AGENT_SESSION_BIN", path.to_string_lossy())
        .env("AGENT_SESSION_ID", "successor-session")
}

fn takeover(
    stub: &StubEnv,
    head: &str,
    base: &str,
    tip: &str,
    selector: &str,
    dry: bool,
) -> super::support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry {
        args.push("--dry-run");
    }
    args.extend([
        "pr",
        "review-handoff",
        "recover",
        "7",
        "--expected-head",
        head,
        "--expected-state",
        tip,
        "--base-sha",
        base,
        "--reason",
        "coordinator-retired",
        "--coordinator-session",
        selector,
    ]);
    run_forge_cli(stub, &args)
}

#[test]
fn retired_coordinator_takeover_preview_transfers_only_coordinator_authority() {
    let seeded = records(retired_coordinator_handoff(), Some(HEAD));
    let tip = &seeded.last().unwrap().record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted")
        .env("PROVIDER_BASE", HEAD);
    let out = takeover(
        &stub,
        HEAD,
        HEAD,
        tip,
        &format!("{COORDINATOR_UUID}@source"),
        true,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["state_tip_digest"], *tip);
    assert_eq!(
        data["handoff"]["coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    assert_eq!(
        data["handoff"]["reviewer_digest"],
        review_state::sha256_digest(b"reviewer-session")
    );
    assert_eq!(data["handoff"]["assignment_generation"], 2);
    assert_eq!(data["handoff"]["base_sha"], HEAD);
    assert_eq!(data["handoff"]["returned_reason"], "coordinator-retired");
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["old_coordinator_digest"],
        review_state::sha256_digest(COORDINATOR_UUID.as_bytes())
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["new_coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["reason"],
        "coordinator-retired"
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["lifecycle_source_digest"],
        review_state::sha256_digest(b"source")
    );
    assert!(!out.stdout.contains("\"source\""));
    assert!(!out.stdout.contains(COORDINATOR_UUID));
    assert!(!out.stdout.contains("successor-session"));
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn retired_coordinator_takeover_refuses_live_unproven_and_wrong_identity() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded.last().unwrap().record_digest;
    for (state, reason, selector, kind) in [
        ("live", "", COORDINATOR_UUID, "review_coordinator_live"),
        (
            "stopped",
            "",
            COORDINATOR_UUID,
            "review_coordinator_unproven",
        ),
        (
            "closed",
            "vanished",
            COORDINATOR_UUID,
            "review_coordinator_unproven",
        ),
        (
            "closed",
            "deleted",
            OTHER_COORDINATOR_UUID,
            "review_assignment_conflict",
        ),
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), state, reason);
        let out = takeover(&stub, HEAD, OLD, tip, selector, true);
        assert_refusal(&out, 65, kind);
    }
}

#[test]
fn retired_coordinator_takeover_requires_exact_head_base_and_tip() {
    let seeded = records(handoff(None), None);
    let tip = &seeded.last().unwrap().record_digest;
    for (head, base, expected_tip, kind) in [
        (OLD, OLD, tip.as_str(), "review_state_conflict"),
        (HEAD, HEAD, tip.as_str(), "review_scope_changed"),
        (HEAD, OLD, "none", "review_state_conflict"),
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "archived");
        let out = takeover(&stub, head, base, expected_tip, "worker-session", true);
        assert_refusal(&out, 65, kind);
    }
}

/// A provider adapter stores posted bodies and reads them back through trusted comments.
pub(super) fn writable_ledger(stub: StubEnv, seeded: &[ReviewStateRecord]) -> StubEnv {
    let inner = stub.tempdir.path().join("gh-inner");
    fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
    let seed = stub.tempdir.path().join("ledger.json");
    let bodies: Vec<_> = seeded
        .iter()
        .map(|r| review_state::render_state_comment_body(r, None).unwrap())
        .collect();
    fs::write(&seed, serde_json::to_string(&bodies).unwrap()).unwrap();
    let script = r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
ledger = root / 'ledger.json'
if args[:2] == ['api', 'repos/acme/widgets/issues/7/comments']:
    bodies = json.loads(ledger.read_text())
    bodies.append(next(a[5:] for a in args if a.startswith('body=')))
    ledger.write_text(json.dumps(bodies))
    print('https://github.com/acme/widgets/pull/7#issuecomment-2')
elif args[:2] == ['api', 'graphql'] and any('comments(first:' in a for a in args):
    seed_count = int((root / 'seed-count').read_text())
    nodes = [{'author': {'login': 'review-app[bot]'}, 'authorAssociation': 'OWNER',
              'createdAt': '2026-07-20T12:00:03Z' if i >= seed_count else '2026-07-20T12:00:00Z', 'body': b}
             for i, b in enumerate(json.loads(ledger.read_text()))]
    print(json.dumps({'data': {'viewer': {'login': 'review-app[bot]'}, 'repository': {
        'pullRequest': {'comments': {'nodes': nodes, 'pageInfo': {'hasNextPage': False, 'endCursor': None}}}}}}))
else:
    os.execv(str(root / 'gh-inner'), [str(root / 'gh-inner')] + args)
"#;
    fs::write(
        stub.tempdir.path().join("seed-count"),
        seeded.len().to_string(),
    )
    .unwrap();
    stub.gh_stub(script)
}

fn inspect_handoff(stub: &StubEnv) -> super::support::CmdOutput {
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
            "inspect",
            "7",
            "--expected-head",
            HEAD,
        ],
    )
}

fn assign_handoff(
    stub: &StubEnv,
    reviewer: &str,
    base: &str,
    tip: &str,
    dry: bool,
) -> super::support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry {
        args.push("--dry-run");
    }
    args.extend([
        "pr",
        "review-handoff",
        "assign",
        "7",
        "--reviewer-session",
        reviewer,
        "--review-author",
        "review-app[bot]",
        "--base-sha",
        base,
        "--expected-head",
        HEAD,
        "--expected-state",
        tip,
    ]);
    run_forge_cli(stub, &args)
}

#[test]
fn inspect_preserves_ownership_and_exact_tip_after_base_drift() {
    for returned in [None, Some("reviewer-closed")] {
        let seeded = records(handoff(returned), Some(HEAD));
        let stub = fixture(HEAD, &seeded, vec![]).env("PROVIDER_BASE", HEAD);
        let out = inspect_handoff(&stub);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = &parse_envelope(&out.stdout)["data"];
        assert_eq!(data["status"], "review-scope-changed");
        assert_eq!(data["base_sha"], HEAD);
        assert_eq!(data["handoff"]["base_sha"], OLD);
        assert_eq!(
            data["handoff"]["coordinator_digest"],
            review_state::sha256_digest(b"worker-session")
        );
        assert_eq!(
            data["handoff"]["assignment_generation"],
            if returned.is_some() { 2 } else { 1 }
        );
        assert_eq!(
            data["state_tip_digest"],
            seeded.last().unwrap().record_digest
        );
        assert!(!out.stdout.contains("private-machine-canary"));
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn recovered_assignment_accepts_the_tip_returned_by_inspection() {
    for moved_base in [OLD, HEAD] {
        let seeded = records(handoff(None), Some(HEAD));
        let stub = writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded)
            .env("PROVIDER_BASE", moved_base)
            .env("AGENT_REVIEWER_SESSION", "");
        let recovered = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "pr",
                "review-handoff",
                "recover",
                "7",
                "--expected-head",
                HEAD,
                "--expected-state",
                &seeded.last().unwrap().record_digest,
                "--reason",
                "reviewer-closed",
            ],
        );
        assert_eq!(
            recovered.code, 0,
            "{} {}",
            recovered.stdout, recovered.stderr
        );
        let inspected = inspect_handoff(&stub);
        assert_eq!(
            inspected.code, 0,
            "{} {}",
            inspected.stdout, inspected.stderr
        );
        let data = parse_envelope(&inspected.stdout)["data"].clone();
        assert_eq!(
            data["state_tip_digest"],
            parse_envelope(&recovered.stdout)["data"]["state_tip_digest"]
        );
        let tip = data["state_tip_digest"].as_str().unwrap();
        for dry in [true, false] {
            let out = assign_handoff(&stub, REVIEWER_UUID, moved_base, tip, dry);
            assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
            assert_eq!(
                parse_envelope(&out.stdout)["data"]["handoff"]["assignment_generation"],
                3
            );
        }
    }
}

#[test]
fn same_reviewer_base_refresh_preserves_findings_and_requires_fresh_observation() {
    let mut h = handoff(None);
    h.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let mut seeded = records(h, None);
    let findings: Vec<review_state::ReviewFindingObservation> = serde_json::from_value(json!([
        {"fingerprint":"testing:handoff:open-finding", "blocking":true, "threads":["thread-1"]}
    ]))
    .unwrap();
    let inherited = review_state::observe_review_loop(None, HEAD, &findings)
        .unwrap()
        .state;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(seeded[0].record_digest.clone()),
            ReviewStatePayload::ReviewLoop {
                state: inherited.clone(),
            },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let stub = writable_ledger(fixture(HEAD, &seeded, vec![review(HEAD, "pass")]), &seeded)
        .env("PROVIDER_BASE", HEAD)
        .env("AGENT_REVIEWER_SESSION", "");
    let tip = &seeded.last().unwrap().record_digest;
    for dry in [true, false] {
        let out = assign_handoff(&stub, REVIEWER_UUID, HEAD, tip, dry);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout)["data"].clone();
        assert_eq!(data["handoff"]["assignment_generation"], 2);
        assert_eq!(data["handoff"]["base_sha"], HEAD);
    }
    let saved: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&inherited)
    );
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    let reviewer = stub.env("AGENT_SESSION_ID", REVIEWER_UUID);
    let stale = observe(&reviewer, HEAD);
    assert!(
        stale
            .stdout
            .contains("review_assignment_generation_conflict"),
        "{}",
        stale.stdout
    );
    let reviewer = reviewer.env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let file = reviewer.tempdir.path().join("findings.json");
    fs::write(&file, serde_json::to_vec(&findings).unwrap()).unwrap();
    let out = run_forge_cli(
        &reviewer,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            HEAD,
            "--auto-state",
            "--findings-file",
            file.to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["data"]["appended"], true);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["state"]["findings"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_refusal(&check(&reviewer, HEAD), 65, "awaiting_designated_review");
}

#[test]
fn surrendered_assignment_accepts_current_base_and_inspected_tip() {
    let mut seeded = records(handoff(None), Some(HEAD));
    let mut h = handoff(None);
    h.surrendered = true;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            seeded.len() as u64,
            Some(seeded.last().unwrap().record_digest.clone()),
            ReviewStatePayload::ReviewHandoff { handoff: h },
        )
        .unwrap(),
    );
    let stub = writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded)
        .env("PROVIDER_BASE", HEAD)
        .env("AGENT_REVIEWER_SESSION", "");
    let inspected = inspect_handoff(&stub);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    let data = parse_envelope(&inspected.stdout)["data"].clone();
    assert_eq!(data["handoff"]["surrendered"], true);
    let out = assign_handoff(
        &stub,
        REVIEWER_UUID,
        HEAD,
        data["state_tip_digest"].as_str().unwrap(),
        false,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["handoff"]["assignment_generation"],
        2
    );
}

#[test]
fn scope_refresh_requires_publication_after_the_new_handoff() {
    let mut h = handoff(None);
    h.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let seeded = records(h.clone(), Some(HEAD));
    h.base_sha = HEAD.into();
    h.assignment_generation = 2;
    let refresh = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        2,
        Some(seeded.last().unwrap().record_digest.clone()),
        ReviewStatePayload::ReviewHandoff { handoff: h },
    )
    .unwrap();
    for (published_after_refresh, bound) in [(false, true), (true, false), (true, true)] {
        let mut r = review(HEAD, "pass");
        if bound {
            r["body"] = json!(format!(
                "{}\n<!-- forge-cli:review-handoff:v1 {} -->",
                r["body"].as_str().unwrap(),
                refresh.record_digest
            ));
        }
        if published_after_refresh {
            r["submittedAt"] = json!("2026-07-20T12:00:04Z");
        }
        let stub = writable_ledger(fixture(HEAD, &seeded, vec![r]), &seeded)
            .env("PROVIDER_BASE", HEAD)
            .env("AGENT_REVIEWER_SESSION", "");
        let out = assign_handoff(
            &stub,
            REVIEWER_UUID,
            HEAD,
            &seeded.last().unwrap().record_digest,
            false,
        );
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
        let reviewer = stub
            .env("AGENT_SESSION_ID", REVIEWER_UUID)
            .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
        let findings = reviewer.tempdir.path().join("clean-refresh.json");
        fs::write(&findings, "[]").unwrap();
        let out = run_forge_cli(
            &reviewer,
            &[
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "pr",
                "review-loop",
                "observe",
                "7",
                "--expected-head",
                HEAD,
                "--auto-state",
                "--findings-file",
                findings.to_str().unwrap(),
            ],
        );
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["data"]["state"]["findings"],
            json!({})
        );
        let out = check(&reviewer, HEAD);
        if published_after_refresh && bound {
            assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        } else {
            assert_refusal(&out, 65, "awaiting_designated_review");
        }
    }
}

fn refreshed_publication_records() -> (Vec<ReviewStateRecord>, String, String) {
    let mut h = handoff(None);
    let mut seeded = records(h.clone(), Some(HEAD));
    let old_binding = seeded[0].record_digest.clone();
    h.assignment_generation = 2;
    h.base_sha = HEAD.into();
    let refresh = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        2,
        Some(seeded.last().unwrap().record_digest.clone()),
        ReviewStatePayload::ReviewHandoff { handoff: h },
    )
    .unwrap();
    let current_binding = refresh.record_digest.clone();
    let state = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        3,
        Some(current_binding.clone()),
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap()
    .with_assignment_generation(Some(2))
    .unwrap();
    seeded.extend([refresh, observation]);
    (seeded, old_binding, current_binding)
}

fn bound_publication(id: u64, at: &str, verdict: &str, binding: &str) -> Value {
    let mut r = review(HEAD, verdict);
    r["id"] = json!(format!("REVIEW_{id}"));
    r["databaseId"] = json!(id);
    r["url"] = json!(format!(
        "https://github.com/acme/widgets/pull/7#pullrequestreview-{id}"
    ));
    r["submittedAt"] = json!(at);
    r["body"] = json!(format!(
        "{}\n<!-- forge-cli:review-handoff:v1 {binding} -->",
        r["body"].as_str().unwrap()
    ));
    r
}

#[test]
fn delayed_old_interval_report_does_not_hide_current_interval_approval() {
    let (seeded, old_binding, current_binding) = refreshed_publication_records();
    let current = bound_publication(1, "2026-07-20T12:00:04Z", "pass", &current_binding);
    let delayed = bound_publication(2, "2026-07-20T12:00:05Z", "pass", &old_binding);
    for reviews in [
        vec![current.clone(), delayed.clone()],
        vec![delayed.clone(), current.clone()],
    ] {
        let stub = fixture(HEAD, &seeded, reviews)
            .env("PROVIDER_BASE", HEAD)
            .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
        let output = check(&stub, HEAD);
        assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
        let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
        assert!(calls.contains("repos/acme/widgets/pulls/7/reviews/2"));
        assert!(calls.contains("repos/acme/widgets/pulls/7/reviews/1"));
    }
}

#[test]
fn newer_current_or_ambiguous_interval_report_cannot_reuse_earlier_pass() {
    let (seeded, old_binding, current_binding) = refreshed_publication_records();
    for invalid in [
        "blocked",
        "malformed-report",
        "unbound",
        "malformed-binding",
        "duplicate",
        "conflicting",
        "unknown",
    ] {
        let passing = bound_publication(1, "2026-07-20T12:00:04Z", "pass", &current_binding);
        let mut newer = bound_publication(2, "2026-07-20T12:00:05Z", "pass", &current_binding);
        let body = newer["body"].as_str().unwrap();
        newer["body"] = json!(match invalid {
            "blocked" => body.replace("- Lens verdict: pass", "- Lens verdict: blocked"),
            "malformed-report" => body.replace("- Lens: testing maintainability", "missing lens"),
            "unbound" => review(HEAD, "pass")["body"].as_str().unwrap().to_string(),
            "malformed-binding" => body.replace(&current_binding, "sha256:not-a-digest"),
            "duplicate" =>
                format!("{body}\n<!-- forge-cli:review-handoff:v1 {current_binding} -->"),
            "conflicting" => format!("{body}\n<!-- forge-cli:review-handoff:v1 {old_binding} -->"),
            "unknown" => body.replace(
                &current_binding,
                &review_state::sha256_digest(b"unknown interval")
            ),
            _ => unreachable!(),
        });
        let stub = fixture(HEAD, &seeded, vec![passing, newer])
            .env("PROVIDER_BASE", HEAD)
            .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
        let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
        assert!(
            !calls.contains("repos/acme/widgets/pulls/7/reviews/1"),
            "{invalid}: {calls}"
        );
    }
}

#[test]
fn old_generation_publication_racing_refresh_cannot_satisfy_the_new_interval() {
    let mut h = handoff(None);
    h.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let seeded = records(h.clone(), Some(HEAD));
    h.assignment_generation = 2;
    h.base_sha = HEAD.into();
    let refresh = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        2,
        Some(seeded.last().unwrap().record_digest.clone()),
        ReviewStatePayload::ReviewHandoff { handoff: h },
    )
    .unwrap();
    let state = review_state::observe_review_loop(None, HEAD, &[])
        .unwrap()
        .state;
    let fresh_observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        3,
        Some(refresh.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap()
    .with_assignment_generation(Some(2))
    .unwrap();
    let stub = writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded)
        .env("PROVIDER_BASE", HEAD)
        .env("AGENT_REVIEWER_SESSION", "")
        .env("AGENT_SESSION_ID", REVIEWER_UUID);
    let inner = stub.tempdir.path().join("ledger-gh");
    fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
    fs::write(
        stub.tempdir.path().join("refresh-records.json"),
        serde_json::to_vec(&[
            review_state::render_state_comment_body(&refresh, None).unwrap(),
            review_state::render_state_comment_body(&fresh_observation, None).unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let r = review(HEAD, "pass");
    fs::write(
        stub.tempdir.path().join("review.json"),
        serde_json::to_vec(&r).unwrap(),
    )
    .unwrap();
    let stub = stub.gh_stub(r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
published = root / 'published-body'
r = json.loads((root / 'review.json').read_text())
if args[:2] == ['api', 'repos/acme/widgets/pulls/7/reviews']:
    # The old command already passed its ownership read. Refresh lands just
    # before its native report is published, and a fresh clean observation follows.
    if not published.exists():
        ledger = root / 'ledger.json'
        bodies = json.loads(ledger.read_text()) + json.loads((root / 'refresh-records.json').read_text())
        ledger.write_text(json.dumps(bodies))
    published.write_text(next(a[5:] for a in args if a.startswith('body=')))
    print(r['url'])
elif args[:2] == ['api', 'graphql'] and 'reviews(first:' in ' '.join(args) and 'states: [PENDING]' not in ' '.join(args) and published.exists():
    r['body'] = published.read_text()
    r['submittedAt'] = '2026-07-20T12:00:04Z'
    print(json.dumps({'data': {'viewer': {'login':'review-app[bot]'}, 'repository': {'pullRequest': {
        'headRefOid': r['commit']['oid'], 'reviews': {'nodes':[r], 'pageInfo': {'hasNextPage':False,'endCursor':None}}}}}}))
elif args[:2] == ['api', 'repos/acme/widgets/pulls/7/reviews/1'] and published.exists():
    print(json.dumps({'id':1, 'html_url':r['url'], 'state':r['state'], 'commit_id':r['commit']['oid'],
        'user':{'login':'review-app[bot]', 'type':'Bot', 'node_id':'BOT_REVIEW_APP'}, 'body':published.read_text()}))
else:
    os.execv(str(root / 'ledger-gh'), [str(root / 'ledger-gh')] + args)
"#);
    let body = stub.tempdir.path().join("report.md");
    fs::write(&body, r["body"].as_str().unwrap()).unwrap();
    let args = [
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "pr",
        "review",
        "7",
        "--submit-review",
        "--decision",
        "comments-only",
        "--expected-head",
        HEAD,
        "--comment-file",
        body.to_str().unwrap(),
        "--lens",
        "testing",
    ];
    let out = run_forge_cli(&stub, &args);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(
        fs::read_to_string(stub.tempdir.path().join("published-body"))
            .unwrap()
            .contains(&seeded[0].record_digest)
    );
    assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    let reviewer = stub.env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let out = run_forge_cli(&reviewer, &args);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(
        fs::read_to_string(reviewer.tempdir.path().join("published-body"))
            .unwrap()
            .contains(&refresh.record_digest)
    );
    let out = check(&reviewer, HEAD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
}

#[test]
fn base_refresh_refuses_different_reviewer_unchanged_base_and_stale_tip() {
    let mut h = handoff(None);
    h.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let seeded = records(h, Some(HEAD));
    let tip = &seeded.last().unwrap().record_digest;
    for (reviewer, base, expected_tip, actor, kind) in [
        (
            OTHER_COORDINATOR_UUID,
            HEAD,
            tip.as_str(),
            "worker-session",
            "review_handover_required",
        ),
        (
            REVIEWER_UUID,
            OLD,
            tip.as_str(),
            "worker-session",
            "review_handover_required",
        ),
        (
            REVIEWER_UUID,
            HEAD,
            "none",
            "worker-session",
            "review_state_conflict",
        ),
        (
            REVIEWER_UUID,
            HEAD,
            tip.as_str(),
            "another-worker",
            "review_writer_conflict",
        ),
    ] {
        let stub = fixture(HEAD, &seeded, vec![])
            .env("PROVIDER_BASE", base)
            .env("AGENT_REVIEWER_SESSION", "")
            .env("AGENT_SESSION_ID", actor);
        assert_refusal(
            &assign_handoff(&stub, reviewer, base, expected_tip, true),
            65,
            kind,
        );
    }
}

#[test]
fn assignment_rechecks_provider_head_base_and_tip_before_posting() {
    let mut h = handoff(None);
    h.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let seeded = records(h, Some(HEAD));
    for changed in ["head", "base", "tip"] {
        let stub = fixture(HEAD, &seeded, vec![])
            .env("PROVIDER_BASE", HEAD)
            .env("AGENT_REVIEWER_SESSION", "");
        let inner = stub.tempdir.path().join("gh-inner");
        fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
        let script = format!(
            r#"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
kind = {changed:?}
is_target = (kind == 'head' and args[:2] == ['pr', 'view']) or (kind == 'base' and args[:2] == ['api', 'repos/acme/widgets/pulls/7']) or (kind == 'tip' and 'comments(first:' in ' '.join(args))
counter = root / 'scope-read'
if is_target and counter.exists():
    if kind == 'base': print('{OLD}')
    else:
        result = subprocess.run([str(root / 'gh-inner')] + args, capture_output=True, text=True, check=True)
        data = json.loads(result.stdout)
        if kind == 'head': data['headRefOid'] = '{OLD}'
        else: data['data']['repository']['pullRequest']['comments']['nodes'].pop()
        print(json.dumps(data))
    sys.exit(0)
if is_target: counter.touch()
os.execv(str(root / 'gh-inner'), [str(root / 'gh-inner')] + args)
"#
        );
        let stub = stub.gh_stub(&script);
        let out = assign_handoff(
            &stub,
            REVIEWER_UUID,
            HEAD,
            &seeded.last().unwrap().record_digest,
            false,
        );
        assert_refusal(
            &out,
            65,
            if changed == "base" {
                "review_scope_changed"
            } else {
                "review_state_conflict"
            },
        );
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn retired_coordinator_takeover_persists_findings_and_allows_reassignment() {
    let mut seeded = records(retired_coordinator_handoff(), None);
    let findings: Vec<review_state::ReviewFindingObservation> = serde_json::from_value(json!([
        {"fingerprint":"testing:handoff:open-finding", "blocking":true, "threads":["thread-1"]}
    ]))
    .unwrap();
    let original = review_state::observe_review_loop(None, HEAD, &findings)
        .unwrap()
        .state;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(seeded[0].record_digest.clone()),
            ReviewStatePayload::ReviewLoop {
                state: original.clone(),
            },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let mut surrendered = retired_coordinator_handoff();
    surrendered.surrendered = true;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            2,
            Some(seeded[1].record_digest.clone()),
            ReviewStatePayload::ReviewHandoff {
                handoff: surrendered,
            },
        )
        .unwrap(),
    );
    let stub = writable_ledger(
        coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "archived"),
        &seeded,
    )
    .env("PROVIDER_BASE", HEAD)
    .env("AGENT_REVIEWER_SESSION", "");
    let out = takeover(
        &stub,
        HEAD,
        HEAD,
        &seeded.last().unwrap().record_digest,
        &format!("{COORDINATOR_UUID}@source"),
        false,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let tip = parse_envelope(&out.stdout)["data"]["state_tip_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let saved: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), seeded.len() + 1);
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&original)
    );
    assert!(
        saved
            .last()
            .unwrap()
            .contains("Coordinator ownership transferred")
    );
    // Coordinator transfer alone never authorizes reviewer appends or publication.
    let blocked = observe(&stub, HEAD);
    assert_eq!(
        parse_envelope(&blocked.stdout)["data"]["preflight_ok"],
        false
    );
    assert!(blocked.stdout.contains("designated_reviewer_unavailable"));
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "assign",
            "7",
            "--reviewer-session",
            REVIEWER_UUID,
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            HEAD,
            "--expected-head",
            HEAD,
            "--expected-state",
            &tip,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["handoff"]["assignment_generation"], 3);
    assert!(data["handoff"].get("coordinator_transfer").is_none());
    let blocked = observe(&stub, HEAD);
    assert_eq!(
        parse_envelope(&blocked.stdout)["data"]["preflight_ok"],
        false
    );
    assert!(blocked.stdout.contains("review_writer_conflict"));
    let published = run_forge_cli(
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
    assert_refusal(&published, 65, "review_writer_conflict");
    let reviewer = stub
        .env("AGENT_SESSION_ID", REVIEWER_UUID)
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "3");
    let findings_file = reviewer.tempdir.path().join("open-findings.json");
    fs::write(&findings_file, r#"[{"lifecycle_fingerprint":"testing:handoff:open-finding","disposition":"open","blocking":true,"threads":["thread-1"]}]"#).unwrap();
    let out = run_forge_cli(
        &reviewer,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            HEAD,
            "--auto-state",
            "--findings-file",
            findings_file.to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["appended"], true, "{}", out.stdout);
    let saved: Vec<String> = serde_json::from_str(
        &fs::read_to_string(reviewer.tempdir.path().join("ledger.json")).unwrap(),
    )
    .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&original)
    );
}

#[test]
fn retired_coordinator_takeover_rechecks_lifecycle_before_posting() {
    let seeded = records(retired_coordinator_handoff(), None);
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
    let inner = stub.tempdir.path().join("board-inner");
    fs::rename(stub.tempdir.path().join("agent-session"), &inner).unwrap();
    let counter = stub.tempdir.path().join("board-read");
    let script = format!(
        r#"#!/bin/sh
if [ -e '{counter}' ]; then
  '{inner}' "$@" | sed 's/"closed"/"live"/g'
else
  touch '{counter}'
  exec '{inner}' "$@"
fi
"#,
        counter = counter.display(),
        inner = inner.display()
    );
    stub.write_stub("agent-session", &script);
    let out = takeover(
        &stub,
        HEAD,
        OLD,
        &seeded[0].record_digest,
        COORDINATOR_UUID,
        false,
    );
    assert_refusal(&out, 65, "review_coordinator_live");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn retired_coordinator_takeover_refuses_incomplete_or_stale_board_evidence() {
    let seeded = records(retired_coordinator_handoff(), None);
    for change in [
        "absent",
        "offline",
        "truncated",
        "stale",
        "malformed",
        "failed",
        "duplicate-live",
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let now = jiff::Timestamp::now().to_string();
        let mut board = json!({"ok":true,"data":{"board":{
            "schema_version":"agent-session.board-view.v1", "generated_at":now, "truncated":false,
            "machines":[{"machine":"source", "available":true, "last_seen_at":now}],
            "records":[{"session_id":COORDINATOR_UUID, "machine":"source", "state":"closed",
                "runtime_status":null,"closed_at":now,"close_reason":"deleted"}]
        }}});
        let b = &mut board["data"]["board"];
        match change {
            "absent" => b["records"] = json!([]),
            "offline" => b["machines"][0]["available"] = json!(false),
            "truncated" => b["truncated"] = json!(true),
            "stale" => b["machines"][0]["last_seen_at"] = json!("2020-01-01T00:00:00Z"),
            "duplicate-live" => b["records"]
                .as_array_mut()
                .unwrap()
                .push(json!({"session_id":COORDINATOR_UUID,"state":"live"})),
            _ => (),
        }
        let script = match change {
            "malformed" => "#!/bin/sh\necho private-evidence-canary\n".into(),
            "failed" => "#!/bin/sh\necho private-evidence-canary >&2\nexit 1\n".into(),
            _ => format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
        };
        stub.write_stub("agent-session", &script);
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            COORDINATOR_UUID,
            true,
        );
        let kind = if change == "duplicate-live" {
            "review_coordinator_live"
        } else {
            "review_coordinator_unproven"
        };
        assert_refusal(&out, 65, kind);
        let error = parse_envelope(&out.stdout)["error"].clone();
        assert!(error["details"]["retryable"].is_boolean());
        assert!(error["details"]["next_action"].is_string());
        assert!(error["details"]["recovery"].is_object());
        assert!(!out.stdout.contains("private-evidence-canary"));
        assert!(!out.stderr.contains("private-evidence-canary"));
    }
}

#[test]
fn retired_coordinator_takeover_chain_validates_transfer_and_racing_writers() {
    let root = records(handoff(None), None).remove(0);
    let mut value = serde_json::to_value(handoff(None)).unwrap();
    value["coordinator_digest"] = json!(review_state::sha256_digest(b"successor-session"));
    value["returned_reason"] = json!("coordinator-retired");
    value["assignment_generation"] = json!(2);
    value["coordinator_transfer"] = json!({
        "old_coordinator_digest":review_state::sha256_digest(b"worker-session"),
        "new_coordinator_digest":review_state::sha256_digest(b"successor-session"),
        "lifecycle_source_digest":review_state::sha256_digest(b"source"),
        "reason":"coordinator-retired"
    });
    let transfer = |v: Value| {
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(root.record_digest.clone()),
            ReviewStatePayload::ReviewHandoff {
                handoff: serde_json::from_value(v).unwrap(),
            },
        )
        .unwrap()
    };
    let recovery = transfer(value.clone());
    let observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewLoop {
            state: review_state::observe_review_loop(None, HEAD, &[])
                .unwrap()
                .state,
        },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    let markers = [
        root.marker().unwrap(),
        observation.marker().unwrap(),
        recovery.marker().unwrap(),
    ];
    let chain =
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.tip_digest, Some(recovery.record_digest.clone()));
    assert_eq!(chain.ignored_stale_records, vec![observation.record_digest]);
    let mut invalid = value.clone();
    invalid["coordinator_transfer"]["old_coordinator_digest"] =
        json!(review_state::sha256_digest(b"unrelated-session"));
    let markers = [root.marker().unwrap(), transfer(invalid).marker().unwrap()];
    assert!(
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).is_err()
    );
    let mut competing = value;
    competing["coordinator_digest"] = json!(review_state::sha256_digest(b"another-successor"));
    competing["coordinator_transfer"]["new_coordinator_digest"] =
        competing["coordinator_digest"].clone();
    let markers = [
        root.marker().unwrap(),
        recovery.marker().unwrap(),
        transfer(competing).marker().unwrap(),
    ];
    assert!(
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).is_err()
    );
    assert!(
        !serde_json::to_string(&handoff(None))
            .unwrap()
            .contains("coordinator_transfer")
    );
}

#[test]
fn retired_coordinator_takeover_rechecks_provider_scope_before_posting() {
    let seeded = records(retired_coordinator_handoff(), None);
    for changed in ["head", "base", "tip"] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let inner = stub.tempdir.path().join("gh-inner");
        fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
        let counter = stub.tempdir.path().join("scope-read");
        let script = format!(
            r#"#!/bin/sh
case "$1 $2" in
  "pr view")
    if [ '{changed}' = head ]; then
      if [ -e '{counter}' ]; then '{inner}' "$@" | sed 's/{HEAD}/{OLD}/g'; exit 0; fi
      touch '{counter}'
    fi ;;
  "api repos/acme/widgets/pulls/7")
    if [ '{changed}' = base ]; then
      if [ -e '{counter}' ]; then echo '{HEAD}'; exit 0; fi
      touch '{counter}'
    fi ;;
  "api graphql")
    if [ '{changed}' = tip ]; then
      if [ -e '{counter}' ]; then
        echo '{{"data":{{"viewer":{{"login":"review-app[bot]"}},"repository":{{"pullRequest":{{"comments":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}}}'
        exit 0
      fi
      touch '{counter}'
    fi ;;
esac
exec '{inner}' "$@"
"#,
            changed = changed,
            counter = counter.display(),
            inner = inner.display()
        );
        let stub = stub.gh_stub(&script);
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            COORDINATOR_UUID,
            false,
        );
        assert_refusal(
            &out,
            65,
            if changed == "base" {
                "review_scope_changed"
            } else {
                "review_state_conflict"
            },
        );
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn retired_coordinator_takeover_binds_the_selected_lifecycle_source() {
    let seeded = records(retired_coordinator_handoff(), None);
    for scenario in ["unavailable", "live-namesake", "closed-namesake"] {
        let target_closed = scenario != "unavailable";
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let now = jiff::Timestamp::now().to_string();
        let mut board = json!({"ok":true,"data":{"board":{
            "schema_version":"agent-session.board-view.v1", "generated_at":now, "truncated":false,
            "machines":[{"machine":"target", "available":target_closed, "last_seen_at":now},
                {"machine":"namesake", "available":true, "last_seen_at":now}],
            "records":if target_closed { json!([
                {"session_id":COORDINATOR_UUID, "machine":"target", "state":"closed", "runtime_status":null,
                    "closed_at":now,"close_reason":"deleted"},
                {"session_id":COORDINATOR_UUID, "machine":"namesake", "state":"live", "runtime_status":"running"}
            ]) } else { json!([
                {"session_id":COORDINATOR_UUID, "machine":"namesake", "state":"closed", "runtime_status":null,
                    "closed_at":now,"close_reason":"deleted"}
            ]) }
        }}});
        if scenario == "closed-namesake" {
            board["data"]["board"]["records"][1] = json!({
                "session_id":COORDINATOR_UUID, "machine":"namesake", "state":"closed", "runtime_status":null,
                "closed_at":now,"close_reason":"deleted"
            });
        }
        stub.write_stub(
            "agent-session",
            &format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
        );
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            &format!("{COORDINATOR_UUID}@target"),
            true,
        );
        if scenario == "live-namesake" {
            assert_refusal(&out, 65, "review_coordinator_live");
        } else {
            assert_refusal(&out, 65, "review_coordinator_unproven");
        }
    }
}

#[test]
fn rejected_published_reviewable_names_review_and_field() {
    let mut malformed = review(HEAD, "pass");
    malformed["body"] = json!(
        malformed["body"]
            .as_str()
            .unwrap()
            .replace("Reviewable: PR #7", "Reviewable: PR #7 at aaaaaaa")
    );
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![malformed]);
    let out = check(&stub, HEAD);
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["error"]["code"], "awaiting_designated_review");
    let error = env["error"].to_string();
    assert!(error.contains("Reviewable"), "{error}");
    assert!(error.contains("REVIEW_1"), "{error}");
}

#[test]
fn exact_head_qualified_reviewable_admits_app_approval_in_either_observation_order() {
    for reference in [
        "PR #7",
        "#7",
        "acme/widgets#7",
        "https://github.com/acme/widgets/pull/7",
    ] {
        for submitted_at in ["2026-07-20T12:00:02Z", "2026-07-20T12:00:04Z"] {
            let (mut graphql, mut rest) = app_review_pair();
            let body = graphql["body"].as_str().unwrap().replace(
                "Reviewable: PR #7",
                &format!("Reviewable: {reference} at {HEAD}"),
            );
            graphql["body"] = json!(body);
            graphql["submittedAt"] = json!(submitted_at);
            rest["body"] = graphql["body"].clone();
            let stub = fixture_with_native(
                HEAD,
                &records(handoff(None), Some(HEAD)),
                vec![graphql],
                Some(rest),
            );
            let out = check(&stub, HEAD);
            assert_eq!(out.code, 0, "{reference}: {} {}", out.stdout, out.stderr);
            assert_eq!(parse_envelope(&out.stdout)["data"]["status"], "reviewed");
        }
    }
}

#[test]
fn qualified_reviewable_must_name_the_native_review_head() {
    for reference in [
        format!("PR #7 at {OLD}"),
        format!("PR #8 at {HEAD}"),
        format!("acme/other#7 at {HEAD}"),
        format!("PR #7 at {HEAD} extra"),
        "PR #7 at aaaaaaa".into(),
    ] {
        let (mut graphql, mut rest) = app_review_pair();
        graphql["body"] = json!(
            graphql["body"]
                .as_str()
                .unwrap()
                .replace("Reviewable: PR #7", &format!("Reviewable: {reference}"),)
        );
        rest["body"] = graphql["body"].clone();
        let stub = fixture_with_native(
            HEAD,
            &records(handoff(None), Some(HEAD)),
            vec![graphql],
            Some(rest),
        );
        assert_refusal(&check(&stub, HEAD), 65, "awaiting_designated_review");
    }
}
