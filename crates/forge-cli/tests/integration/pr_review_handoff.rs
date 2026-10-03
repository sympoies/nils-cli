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
        assignment_generation: if returned.is_some() { 2 } else { 1 },
        surrendered: false,
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
  "api repos/acme/widgets/pulls/7") echo "${{PROVIDER_BASE:-{OLD}}}"; exit 0 ;;
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
