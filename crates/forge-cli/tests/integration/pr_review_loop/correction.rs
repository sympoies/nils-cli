use super::*;
use forge_cli::ops::review_state::{
    ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
    observe_review_loop,
};
use pretty_assertions::assert_eq;

#[test]
fn dry_run_accepts_chain_bound_omission_correction_without_reopen_budget() {
    let stub = StubEnv::new();
    let initial: Vec<_> = (1..=5)
        .map(|index| ReviewFindingObservation {
            fingerprint: format!("correctness:inventory:f{index}"),
            root_cause_fingerprint: None,
            blocking: true,
            status: ReviewFindingStatus::Open,
            threads: Vec::new(),
        })
        .collect();
    let state = observe_review_loop(None, "head-1", &initial).unwrap().state;
    let genesis = ReviewStateRecord::new(
        "acme/widgets",
        7,
        "head-1",
        0,
        None,
        ReviewStatePayload::ReviewLoop {
            state: state.clone(),
        },
    )
    .unwrap();
    // Reconstruct the previous omission bug independently of the transition
    // under test, so this immutable history fixture survives the repair.
    let mut erroneous = state;
    erroneous.head_sha = "head-2".into();
    erroneous.round = 1;
    for finding in erroneous.findings.values_mut() {
        finding.status = ReviewFindingStatus::Fixed;
    }
    let fixed = erroneous
        .findings
        .get_mut("correctness:inventory:f5")
        .unwrap();
    fixed.blocking = false;
    fixed.last_seen_head = "head-2".into();
    fixed.seen_count = 2;
    let erroneous_record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        "head-2",
        1,
        Some(genesis.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state: erroneous },
    )
    .unwrap();
    let mut corrected = initial;
    corrected[4].status = ReviewFindingStatus::Fixed;
    corrected[4].blocking = false;
    let body = serde_json::json!({
        "schema_version": "forge-cli.review-loop-correction.v1",
        "correction": {
            "corrected_record": erroneous_record.record_digest,
            "reason": "omitted-findings"
        },
        "findings": corrected
    });
    let path = write_findings(&stub, "correction.json", &body.to_string());
    let script = gh_stub_with_ledger(
        &stub,
        "head-2",
        &[
            genesis.marker().unwrap(),
            erroneous_record.marker().unwrap(),
        ],
    );
    let stub = stub.gh_stub(&script);
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--dry-run",
            "--expected-head",
            "head-2",
            "--expected-state",
            &erroneous_record.record_digest,
            "--findings-file",
            &path,
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], true, "{envelope}");
    assert_eq!(envelope["data"]["would_append"], true);
    assert!(
        envelope["data"]["planned_comment"]["visible_metadata"]
            .as_str()
            .unwrap()
            .contains("correction")
    );
    let log = gh_args_log(&stub);
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(!log.contains(marker), "unexpected mutation: {log}");
    }
}
