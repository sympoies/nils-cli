//! Recovery guidance for an already extended stop survives reviewer rebinding.
use super::*;
use forge_cli::ops::review_state::{
    self, ReviewFindingObservation, ReviewStatePayload, ReviewStateRecord,
};
use pretty_assertions::assert_eq;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn append(
    records: &mut Vec<ReviewStateRecord>,
    head: &str,
    payload: ReviewStatePayload,
    generation: u64,
) {
    records.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            head,
            records.len() as u64,
            records.last().map(|r| r.record_digest.clone()),
            payload,
        )
        .unwrap()
        .with_assignment_generation(Some(generation))
        .unwrap(),
    );
}

fn rebound(head: &str) -> (Vec<ReviewStateRecord>, Vec<ReviewFindingObservation>) {
    let (mut records, rows) = super::supersession::stopped_fixture();
    let mut owner = super::super::pr_review_handoff::handoff(None);
    append(
        &mut records,
        A,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        1,
    );
    owner.surrendered = true;
    append(
        &mut records,
        A,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        1,
    );
    owner.surrendered = false;
    owner.assignment_generation = 2;
    owner.assigned_head = head.into();
    owner.base_sha = "2222222222222222222222222222222222222222".into();
    append(
        &mut records,
        head,
        ReviewStatePayload::ReviewHandoff { handoff: owner },
        2,
    );
    // Parse the whole chain: assignment changes must not reset the stopped state.
    let chain = super::supersession::chain(&records);
    let state = review_state::latest_review_loop_state(&chain).unwrap();
    assert!(state.hard_stop.as_ref().unwrap().extension_applied);
    assert_eq!(state.extensions.len(), 1);
    (records, rows)
}

fn fixture(head: &str, records: &[ReviewStateRecord]) -> StubEnv {
    super::super::pr_review_handoff::fixture(head, records, vec![])
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2")
}

#[test]
fn applied_extension_rebind_mismatch_names_supported_recovery() {
    for head in [A, B] {
        let (records, mut rows) = rebound(head);
        if head == A {
            rows[0].threads.push("thread-2".into());
        }
        let stub = fixture(head, &records);
        let file = write_findings(
            &stub,
            "changed.json",
            &serde_json::to_string(&rows).unwrap(),
        );
        let out = run_forge_cli(
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
                head,
                "--expected-state",
                &records.last().unwrap().record_digest,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["preflight_ok"], false);
        for rule in [
            "findings_file",
            "provider_pull_request",
            "expected_head",
            "review_state_chain",
            "designated_review_writer",
            "expected_state_tip",
        ] {
            assert_eq!(preflight_verdict(&envelope, rule)["ok"], true, "{envelope}");
        }
        let verdict = preflight_verdict(&envelope, "observation_transition");
        assert_eq!(verdict["code"], "review_state_conflict");
        let diagnostic = verdict.to_string();
        assert!(
            diagnostic.contains("forge-cli pr review-loop inspect"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("forge-cli pr review-loop observe"),
            "{diagnostic}"
        );
        if head == A {
            assert!(diagnostic.contains("original-findings"), "{diagnostic}");
            assert!(diagnostic.contains("observation_digest"), "{diagnostic}");
        } else {
            assert!(
                diagnostic.contains("forge-cli.review-loop-supersession.v1"),
                "{diagnostic}"
            );
            assert!(diagnostic.contains("signed-fast-forward"), "{diagnostic}");
            assert!(
                diagnostic.contains("forge-cli pr review-loop extend"),
                "{diagnostic}"
            );
            assert!(
                diagnostic.contains("fresh maintainer approval"),
                "{diagnostic}"
            );
        }
        let live = run_forge_cli(
            &stub,
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
                head,
                "--expected-state",
                &records.last().unwrap().record_digest,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(live.code, 65, "{} {}", live.stderr, live.stdout);
        let error = parse_envelope(&live.stdout);
        assert_eq!(error["error"]["code"], "review_state_conflict");
        assert_eq!(error["error"]["message"], verdict["message"]);
        let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
        for marker in PROVIDER_WRITE_MARKERS {
            assert!(!calls.contains(marker), "{calls}");
        }
    }
}

#[test]
fn applied_extension_rebind_original_inventory_resumes_without_extra_budget() {
    let (records, rows) = rebound(A);
    let chain = super::supersession::chain(&records);
    let previous = review_state::latest_review_loop_state(&chain).unwrap();
    let snapshot = previous.clone();
    let resumed = review_state::observe_review_loop(Some(previous), A, &rows).unwrap();
    assert!(resumed.changed);
    assert!(resumed.state.hard_stop.is_none());
    assert_eq!(resumed.state.round, previous.round + 1);
    assert_eq!(resumed.state.budget, previous.budget);
    assert_eq!(resumed.state.extensions, previous.extensions);
    assert_eq!(previous, &snapshot);

    let stub = fixture(A, &records);
    let file = write_findings(
        &stub,
        "original.json",
        &serde_json::to_string(&rows).unwrap(),
    );
    let out = run_forge_cli(
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
            A,
            "--expected-state",
            &records.last().unwrap().record_digest,
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], true, "{envelope}");
    assert_eq!(envelope["data"]["would_append"], true);
}

#[test]
fn applied_extension_rebind_supersession_preserves_history_and_requires_explicit_extension() {
    use review_state::supersession::{self, SupersessionReason, SupersessionRequest};
    let (mut records, rows) = rebound(B);
    let prefix: Vec<_> = records
        .iter()
        .map(|record| record.marker().unwrap())
        .collect();
    let chain = super::supersession::chain(&records);
    let previous = review_state::latest_review_loop_state(&chain)
        .unwrap()
        .clone();
    let request = SupersessionRequest {
        // The last state record is the consumed extension, before the handoffs.
        superseded_record: records[2].record_digest.clone(),
        reason: SupersessionReason::SignedFastForward,
    };
    let superseded = supersession::apply(
        &chain,
        B,
        chain.tip_digest.as_deref(),
        &request,
        &rows,
        &[B.into()],
    )
    .unwrap();
    let stop = superseded.state.hard_stop.as_ref().unwrap().clone();
    assert!(!stop.extension_applied);
    assert_eq!(superseded.state.budget, previous.budget);
    assert_eq!(superseded.state.extensions, previous.extensions);
    assert_eq!(superseded.state.findings, previous.findings);
    assert_eq!(superseded.state.round, previous.round);
    assert_eq!(
        superseded.state.no_progress_rounds,
        previous.no_progress_rounds
    );
    assert_eq!(
        review_state::observe_review_loop(Some(&superseded.state), B, &rows)
            .unwrap_err()
            .kind(),
        stop.code
    );
    append(&mut records, B, superseded.into_payload(), 2);
    let chain = super::supersession::chain(&records);
    let extended = review_state::apply_review_loop_extension(
        review_state::latest_review_loop_state(&chain).unwrap(),
        stop.proposal_digest,
        "https://github.com/acme/widgets/pull/7#issuecomment-20".into(),
        &stop.code,
        &stop.budget_field,
        stop.increment,
    )
    .unwrap();
    assert_eq!(extended.extensions.len(), previous.extensions.len() + 1);
    assert_eq!(
        extended.budget.max_repair_rounds,
        previous.budget.max_repair_rounds + stop.increment
    );
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewLoop { state: extended },
        2,
    );
    let chain = super::supersession::chain(&records);
    let resumed =
        review_state::observe_review_loop(review_state::latest_review_loop_state(&chain), B, &rows)
            .unwrap();
    assert!(resumed.state.hard_stop.is_none());
    assert_eq!(resumed.state.round, previous.round + 1);
    assert_eq!(resumed.state.findings.len(), previous.findings.len());
    append(&mut records, B, resumed.into_payload(), 2);
    let chain = super::supersession::chain(&records);
    let state = review_state::latest_review_loop_state(&chain).unwrap();
    assert_eq!(state.extensions.len(), 2);
    assert!(state.hard_stop.is_none());
    assert_eq!(
        chain.records[..prefix.len()]
            .iter()
            .map(|record| record.marker().unwrap())
            .collect::<Vec<_>>(),
        prefix
    );
}
