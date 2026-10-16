use forge_cli::ops::{
    pr_review_handoff::ReviewHandoff,
    review_state::{
        ReviewLoopBudget, ReviewLoopHardStop, ReviewLoopState, ReviewStatePayload,
        ReviewStateRecord, sha256_digest,
    },
};
use pretty_assertions::assert_eq;
use std::fs;

use super::{
    pr_merge::{github_merge_stub_full, make_github_repo, review_loop_marker},
    support::{StubEnv, parse_envelope, run_forge_cli_in},
};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PASS_CHECKS: &str =
    r#"[{"name":"ci","bucket":"pass","state":"COMPLETED","isRequired":true}]"#;
const DIRECT_POLICY: &str = r#"{"data":{"repository":{"mergeQueue":null,"issues":{"nodes":[]},"pullRequest":{"id":"PR_node7","isInMergeQueue":false}}}}"#;
const QUEUE_POLICY: &str = r#"{"data":{"repository":{"mergeQueue":{"configuration":{"mergeMethod":"SQUASH"}},"issues":{"nodes":[]},"pullRequest":{"id":"PR_node7","isInMergeQueue":false}}}}"#;

fn assignment_record() -> ReviewStateRecord {
    ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewHandoff {
            handoff: ReviewHandoff {
                approval_policy: Default::default(),
                coordinator_digest: sha256_digest(b"worker-session"),
                reviewer_digest: sha256_digest(b"reviewer-session"),
                review_author: "review-app[bot]".into(),
                base_sha: HEAD.into(),
                assigned_head: HEAD.into(),
                returned_reason: None,
                assignment_generation: 1,
                surrendered: false,
                coordinator_transfer: None,
                base_ref: None,
            },
        },
    )
    .unwrap()
}

fn assignment_without_observation() -> String {
    assignment_record().marker().unwrap()
}

fn assigned_observation_chain() -> String {
    let assignment = assignment_record();
    let observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(assignment.record_digest.clone()),
        ReviewStatePayload::ReviewLoop {
            state: ReviewLoopState {
                head_sha: HEAD.into(),
                round: 0,
                no_progress_rounds: 0,
                budget: ReviewLoopBudget::default(),
                findings: Default::default(),
                extensions: vec![],
                hard_stop: None,
            },
        },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    format!(
        "{}\n{}",
        assignment.marker().unwrap(),
        observation.marker().unwrap()
    )
}

fn ledger_response(markers: &str) -> String {
    let nodes = markers
        .lines()
        .map(|body| {
            serde_json::json!({
                "author": {"login": "maintainer"}, "authorAssociation": "MEMBER",
                "body": body, "createdAt": "2026-07-20T12:00:00Z",
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({"data": {"viewer": {"login": "maintainer"}, "repository": {
        "pullRequest": {"comments": {"nodes": nodes,
            "pageInfo": {"hasNextPage": false, "endCursor": null}}}
    }}})
    .to_string()
}

fn run_case(
    extra: &[&str],
    queue: bool,
    marker: String,
    head: &str,
    expected_code: i32,
    error: Option<&str>,
) {
    run_case_with_later_ledger(
        extra,
        queue,
        marker,
        head,
        expected_code,
        error,
        None,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn run_case_with_later_ledger(
    extra: &[&str],
    queue: bool,
    marker: String,
    head: &str,
    expected_code: i32,
    error: Option<&str>,
    later: Option<String>,
    publish_during_policy_recheck: bool,
) {
    let tempdir = make_github_repo(Some("[review_convergence]\nrequire = true\n"));
    let repo = tempdir.path().join("repo");
    let stub = StubEnv::new();
    let merged = stub.tempdir.path().join("github-merged");
    let enqueued = stub.tempdir.path().join("enqueued");
    let queue_cases = format!(
        r#"
      *"ForgeEnqueuePullRequest"*"expectedHeadOid=head123"*) touch {}; printf '%s\n' '{{"data":{{"enqueuePullRequest":{{"mergeQueueEntry":{{"state":"QUEUED","position":1}}}}}}}}' ;;
      *"ForgeMergeQueuePoll"*) printf '%s\n' '{{"data":{{"repository":{{"pullRequest":{{"state":"MERGED","mergeCommit":{{"oid":"merge456"}},"mergeQueueEntry":null}}}}}}}}' ;;
"#,
        enqueued.display()
    );
    let body = github_merge_stub_full(
        &stub,
        "",
        "",
        !marker.contains('\n'),
        Some(marker.clone()),
        PASS_CHECKS,
        if queue { QUEUE_POLICY } else { DIRECT_POLICY },
        &queue_cases,
    )
    .replace("head123", head)
    .replace(
        "  \"api --paginate\"",
        &format!(
            "  \"api repos/acme/widgets/pulls/7\") printf '%s\\n' '{HEAD}' ;;\n  \"api --paginate\""
        ),
    );
    let first = ledger_response(&marker);
    let state_reads = stub.tempdir.path().join("state-reads");
    let later_json = stub.tempdir.path().join("later-ledger.json");
    fs::write(
        &later_json,
        ledger_response(later.as_deref().unwrap_or(&marker)),
    )
    .unwrap();
    let published = stub.tempdir.path().join("published-observation");
    let policy_reads = stub.tempdir.path().join("policy-reads");
    let policy_case = format!(
        r#"      *"ForgeMergePolicy"*)
        count=0
        if [ -f {count} ]; then count=$(cat {count}); fi
        count=$((count + 1)); printf '%s\n' "$count" > {count}
        if [ "$count" -ge 2 ]; then touch {published}; fi
        printf '%s\n' '{policy}' ;;
"#,
        count = policy_reads.display(),
        published = published.display(),
        policy = if queue { QUEUE_POLICY } else { DIRECT_POLICY }
    );
    let state_case = format!(
        r#"      *"authorAssociation body createdAt"*)
        count=0
        if [ -f {count} ]; then count=$(cat {count}); fi
        count=$((count + 1)); printf '%s\n' "$count" > {count}
        if [ "$count" -ge 2 ]; then cat {later}; else printf '%s\n' '{first}'; fi ;;
"#,
        count = state_reads.display(),
        later = later_json.display()
    );
    let state_case = if publish_during_policy_recheck {
        state_case.replace(
            "if [ \"$count\" -ge 2 ]",
            &format!("if [ -f {} ]", published.display()),
        )
    } else {
        state_case
    };
    let body = body
        .lines()
        .map(|line| {
            if line.contains("*\"authorAssociation body createdAt\"*") {
                state_case.as_str()
            } else if publish_during_policy_recheck && line.contains("*\"ForgeMergePolicy\"*") {
                policy_case.as_str()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let stub = stub.gh_gate_stub(&body);
    let stub = if extra.contains(&"--dry-run") {
        stub.env("AGENT_REVIEWER_SESSION", "reviewer-session")
            .env("AGENT_SESSION_ID", "worker-session")
    } else {
        stub
    };
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "pr",
        "merge",
        "7",
        "--keep-branch",
    ];
    args.extend_from_slice(extra);
    let out = run_forge_cli_in(&stub, &args, Some(&repo));
    assert_eq!(
        out.code, expected_code,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    if let Some(error) = error {
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            error,
            "{}",
            out.stdout
        );
    }
    let expect_mutation = expected_code == 0 && !extra.contains(&"--dry-run");
    assert_eq!(merged.exists(), expect_mutation && !queue);
    assert_eq!(enqueued.exists(), expect_mutation && queue);
}

#[test]
fn explicit_false_allows_assigned_release_without_ledger_direct() {
    run_case(
        &["--review-convergence=false"],
        false,
        assignment_without_observation(),
        HEAD,
        0,
        None,
    );
}

#[test]
fn explicit_false_allows_assigned_release_without_ledger_queue() {
    run_case(
        &["--review-convergence=false"],
        true,
        assignment_without_observation(),
        HEAD,
        0,
        None,
    );
}

#[test]
fn explicit_false_allows_assigned_dry_run_without_ledger() {
    run_case(
        &["--review-convergence=false", "--dry-run"],
        false,
        assignment_without_observation(),
        HEAD,
        0,
        None,
    );
}

#[test]
fn default_still_requires_assigned_review_observation() {
    run_case(
        &[],
        false,
        assignment_without_observation(),
        HEAD,
        65,
        Some("awaiting_designated_review"),
    );
}

#[test]
fn explicit_true_still_requires_assigned_review_observation() {
    run_case(
        &["--review-convergence=true"],
        false,
        assignment_without_observation(),
        HEAD,
        65,
        Some("awaiting_designated_review"),
    );
}

#[test]
fn explicit_false_preserves_existing_blocking_findings() {
    run_case(
        &["--review-convergence=false"],
        false,
        review_loop_marker(true),
        "head123",
        65,
        Some("review_findings_open"),
    );
}

#[test]
fn default_dry_run_still_requires_assigned_review_observation() {
    run_case(
        &["--dry-run"],
        false,
        assignment_without_observation(),
        HEAD,
        65,
        Some("awaiting_designated_review"),
    );
}

#[test]
fn explicit_false_preserves_existing_hard_stop() {
    let marker = ReviewStateRecord::new(
        "acme/widgets",
        7,
        "head123",
        0,
        None,
        ReviewStatePayload::ReviewLoop {
            state: ReviewLoopState {
                head_sha: "head123".into(),
                round: 0,
                no_progress_rounds: 0,
                budget: ReviewLoopBudget::default(),
                findings: Default::default(),
                extensions: vec![],
                hard_stop: Some(ReviewLoopHardStop {
                    code: "review_round_limit_exceeded".into(),
                    budget_field: "max_repair_rounds".into(),
                    increment: 1,
                    proposal_digest: sha256_digest(b"proposal"),
                    attempted_head_sha: "head123".into(),
                    observation_digest: sha256_digest(b"observation"),
                    extension_applied: false,
                }),
            },
        },
    )
    .unwrap()
    .marker()
    .unwrap();
    run_case(
        &["--review-convergence=false"],
        false,
        marker,
        "head123",
        65,
        Some("review_round_limit_exceeded"),
    );
}

#[test]
fn explicit_false_preserves_existing_review_head_binding() {
    run_case(
        &["--review-convergence=false"],
        false,
        review_loop_marker(false),
        HEAD,
        65,
        Some("review_state_conflict"),
    );
}

#[test]
fn explicit_false_preserves_assigned_publication_with_existing_observation() {
    run_case(
        &["--review-convergence=false"],
        false,
        assigned_observation_chain(),
        HEAD,
        65,
        Some("awaiting_designated_review"),
    );
}

#[test]
fn explicit_false_refuses_new_observation_before_direct_merge() {
    run_case_with_later_ledger(
        &["--review-convergence=false"],
        false,
        assignment_without_observation(),
        HEAD,
        65,
        Some("review_state_conflict"),
        Some(assigned_observation_chain()),
        false,
    );
}

#[test]
fn explicit_false_refuses_new_observation_before_enqueue() {
    run_case_with_later_ledger(
        &["--review-convergence=false"],
        true,
        assignment_without_observation(),
        HEAD,
        65,
        Some("review_state_conflict"),
        Some(assigned_observation_chain()),
        false,
    );
}

#[test]
fn explicit_false_refuses_observation_disappearing_during_validation() {
    run_case_with_later_ledger(
        &["--review-convergence=false"],
        false,
        review_loop_marker(false),
        "head123",
        65,
        Some("review_state_conflict"),
        Some(String::new()),
        false,
    );
}

#[test]
fn explicit_false_refuses_observation_published_during_final_policy_read_direct() {
    run_case_with_later_ledger(
        &["--review-convergence=false"],
        false,
        assignment_without_observation(),
        HEAD,
        65,
        Some("review_state_conflict"),
        Some(assigned_observation_chain()),
        true,
    );
}

#[test]
fn explicit_false_refuses_observation_published_during_final_policy_read_queue() {
    run_case_with_later_ledger(
        &["--review-convergence=false"],
        true,
        assignment_without_observation(),
        HEAD,
        65,
        Some("review_state_conflict"),
        Some(assigned_observation_chain()),
        true,
    );
}
