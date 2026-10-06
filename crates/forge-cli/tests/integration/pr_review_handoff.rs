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

fn review(head: &str, verdict: &str) -> Value {
    json!({"id":"REVIEW_1", "databaseId":1,
        "url":"https://github.com/acme/widgets/pull/7#pullrequestreview-1",
        "author":{"login":"review-app[bot]", "__typename":"Bot", "id":"BOT_REVIEW_APP"}, "state":"COMMENTED", "commit":{"oid":head},
        "submittedAt":"2026-07-20T12:00:02Z",
        "body":format!("<!-- agent-kit:specialist-review-report:v1 -->\n## Review Report\n\n- Reviewable: PR #7\n- Lens: testing maintainability\n- Lens verdict: {verdict}\n- Scope: assigned review fixture\n- Evidence reviewed: fixture validation\n\n| Finding | Severity | Confidence | Evidence | Recommendation |\n| --- | --- | ---: | --- | --- |\n| No findings | none | 0.00 | fixture | none |\n"), "viewerDidAuthor":false})
}

fn fixture(head: &str, records: &[ReviewStateRecord], reviews: Vec<Value>) -> StubEnv {
    fixture_with_native(head, records, reviews, None)
}

fn fixture_with_native(
    head: &str,
    records: &[ReviewStateRecord],
    reviews: Vec<Value>,
    native_override: Option<Value>,
) -> StubEnv {
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
        "state":r["state"],"commit_id":r["commit"]["oid"],
        "user":{"login":r["author"]["login"], "type":r["author"]["__typename"], "node_id":r["author"]["id"]},"body":r["body"]})
    });
    let native = native_override.or(native).unwrap_or(Value::Null);
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
  "api repos/acme/widgets/pulls/7/reviews/"*) cat <<'JSON'
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
fn writable_ledger(stub: StubEnv, seeded: &[ReviewStateRecord]) -> StubEnv {
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
    nodes = [{'author': {'login': 'review-app[bot]'}, 'authorAssociation': 'OWNER',
              'createdAt': '2026-07-20T12:00:00Z', 'body': b}
             for b in json.loads(ledger.read_text())]
    print(json.dumps({'data': {'viewer': {'login': 'review-app[bot]'}, 'repository': {
        'pullRequest': {'comments': {'nodes': nodes, 'pageInfo': {'hasNextPage': False, 'endCursor': None}}}}}}))
else:
    os.execv(str(root / 'gh-inner'), [str(root / 'gh-inner')] + args)
"#;
    stub.gh_stub(script)
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
