use super::*;
use forge_cli::ops::review_state::{
    ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
    observe_review_loop, render_state_comment_body,
};
use pretty_assertions::assert_eq;

#[test]
fn merged_fixed_threads_identical_retry_does_not_append_or_duplicate_outcome() {
    let stub = StubEnv::new();
    let mut fixed = ReviewFindingObservation {
        fingerprint: "correctness:inventory:fixed".into(),
        root_cause_fingerprint: None,
        blocking: true,
        status: ReviewFindingStatus::Open,
        threads: vec!["thread-1".into()],
    };
    let initial = observe_review_loop(None, "head-1", &[fixed.clone()])
        .unwrap()
        .state;
    fixed.status = ReviewFindingStatus::Fixed;
    let repaired = observe_review_loop(Some(&initial), "head-2", &[fixed.clone()])
        .unwrap()
        .state;
    fixed.threads = vec!["thread-2".into()];
    let new = ReviewFindingObservation {
        fingerprint: "correctness:inventory:new".into(),
        root_cause_fingerprint: None,
        blocking: true,
        status: ReviewFindingStatus::Open,
        threads: Vec::new(),
    };
    let rows = vec![fixed, new];
    let first = observe_review_loop(Some(&repaired), "head-2", &rows)
        .unwrap()
        .state;
    let outcome = "## Delivery outcome\nFinding inventory updated.";
    let mut comments = Vec::new();
    let mut tip = None;
    for (generation, (head, state)) in
        [("head-1", initial), ("head-2", repaired), ("head-2", first)]
            .into_iter()
            .enumerate()
    {
        let record = ReviewStateRecord::new(
            "acme/widgets",
            7,
            head,
            generation as u64,
            tip,
            ReviewStatePayload::ReviewLoop { state },
        )
        .unwrap();
        comments.push(
            render_state_comment_body(&record, (generation == 2).then_some(outcome)).unwrap(),
        );
        tip = Some(record.record_digest);
    }
    let findings = write_findings(
        &stub,
        "findings.json",
        &serde_json::to_string(&rows).unwrap(),
    );
    let script = gh_stub_with_ledger(&stub, "head-2", &comments);
    let stub = stub.gh_stub(&script);
    let output = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            "head-2",
            "--expected-state",
            tip.as_deref().unwrap(),
            "--findings-file",
            &findings,
            "--body",
            outcome,
        ],
    );
    assert_eq!(
        output.code, 0,
        "stderr={} stdout={}",
        output.stderr, output.stdout
    );
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(envelope["data"]["appended"], false);
    assert_eq!(envelope["data"]["outcome_posted"], true);
    assert_eq!(envelope["data"]["state_tip_digest"], tip.unwrap());
    assert_eq!(
        envelope["data"]["state"]["findings"]["correctness:inventory:new"]["seen_count"],
        1
    );
    assert!(!gh_args_log(&stub).contains("--method POST"));
}
