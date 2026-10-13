//! Closed-set regressions for the admitted correction review findings.
use super::*;
use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::correction::{self, CorrectionRequest};
use forge_cli::ops::review_state::{
    self, ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
};
use pretty_assertions::{assert_eq, assert_ne};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn fixture(
    alias: bool,
) -> (
    Vec<ReviewStateRecord>,
    Vec<ReviewFindingObservation>,
    Vec<ReviewFindingObservation>,
) {
    let initial: Vec<_> = (1..=5)
        .map(|i| ReviewFindingObservation {
            fingerprint: if alias && i == 1 {
                "correctness:inventory:alias-f1".into()
            } else {
                format!("correctness:inventory:f{i}")
            },
            root_cause_fingerprint: (alias && i == 1).then(|| "correctness:inventory:f1".into()),
            status: ReviewFindingStatus::Open,
            blocking: true,
            threads: vec![format!("thread-{i}")],
        })
        .collect();
    let before = review_state::observe_review_loop(None, A, &initial)
        .unwrap()
        .state;
    let mut bad = before.clone();
    bad.head_sha = B.into();
    bad.round = 1;
    for f in bad.findings.values_mut() {
        f.status = ReviewFindingStatus::Fixed;
    }
    let f = bad.findings.get_mut("correctness:inventory:f5").unwrap();
    f.blocking = false;
    f.last_seen_head = B.into();
    f.seen_count = 2;
    let mut records = Vec::new();
    append(
        &mut records,
        A,
        ReviewStatePayload::ReviewLoop { state: before },
        None,
    );
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewLoop { state: bad },
        None,
    );
    let mut attempted = initial;
    attempted[4].status = ReviewFindingStatus::Fixed;
    attempted[4].blocking = false;
    let mut canonical = attempted.clone();
    if alias {
        canonical[0].fingerprint = canonical[0].root_cause_fingerprint.clone().unwrap();
    }
    (records, canonical, attempted)
}

fn append(
    records: &mut Vec<ReviewStateRecord>,
    head: &str,
    payload: ReviewStatePayload,
    owner: Option<u64>,
) {
    let r = ReviewStateRecord::new(
        "acme/widgets",
        7,
        head,
        records.len() as u64,
        records.last().map(|r| r.record_digest.clone()),
        payload,
    )
    .unwrap()
    .with_assignment_generation(owner)
    .unwrap();
    records.push(r);
}
fn chain(records: &[ReviewStateRecord]) -> review_state::ReviewStateChain {
    review_state::parse_chain(
        records
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str),
        "acme/widgets",
        7,
    )
    .unwrap()
}
fn request(target: &ReviewStateRecord) -> CorrectionRequest {
    serde_json::from_value(
        serde_json::json!({"corrected_record":target.record_digest,"reason":"omitted-findings"}),
    )
    .unwrap()
}
fn input(request: &CorrectionRequest, rows: &[ReviewFindingObservation]) -> serde_json::Value {
    serde_json::json!({"schema_version":"forge-cli.review-loop-correction.v1","correction":request,"findings":rows})
}

#[test]
fn correction_retry_after_same_head_reassignment_is_rejected_before_provider_post() {
    let (original, rows, _) = fixture(false);
    let mut records = vec![original[0].clone()];
    let mut owner = ReviewHandoff {
        coordinator_digest: review_state::sha256_digest(b"worker-session"),
        reviewer_digest: review_state::sha256_digest(b"reviewer-session"),
        review_author: "review-app[bot]".into(),
        base_sha: A.into(),
        assigned_head: B.into(),
        returned_reason: None,
        assignment_generation: 1,
        surrendered: false,
        coordinator_transfer: None,
        base_ref: Some("main".into()),
    };
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        Some(1),
    );
    append(&mut records, B, original[1].payload.clone(), Some(1));
    let request = request(records.last().unwrap());
    let history = chain(&records);
    let corrected =
        correction::apply(&history, B, history.tip_digest.as_deref(), &request, &rows).unwrap();
    append(&mut records, B, corrected.into_payload(), Some(1));
    owner.surrendered = true;
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        Some(1),
    );
    owner.surrendered = false;
    owner.assignment_generation = 2;
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewHandoff { handoff: owner },
        Some(2),
    );
    let history = chain(&records);
    let originals: Vec<_> = history
        .records
        .iter()
        .map(|r| r.marker().unwrap())
        .collect();
    let stub = super::super::pr_review_handoff::fixture(B, &records, vec![])
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let file = write_findings(&stub, "retry.json", &input(&request, &rows).to_string());
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
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            B,
            "--expected-state",
            history.tip_digest.as_deref().unwrap(),
            "--findings-file",
            &file,
        ],
    );
    let log = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(
            !log.contains(marker),
            "provider write preceded safe refusal: {log}"
        );
    }
    assert_ne!(out.code, 0, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "review_correction_invalid",
        "{}",
        out.stdout
    );
    assert_eq!(
        chain(&records)
            .records
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>(),
        originals
    );
}

#[test]
fn validate_catalog_declares_the_correction_shape_and_error_it_emits() {
    let catalog: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(include_str!("../../../docs/specs/forge-cli-ops-v1.yaml")).unwrap();
    let op = catalog["operations"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|op| op["id"].as_str() == Some("pr.review-loop.validate"))
        .unwrap();
    let stub = StubEnv::new();
    let (records, rows, _) = fixture(false);
    let file = write_findings(
        &stub,
        "valid.json",
        &input(&request(&records[1]), &rows).to_string(),
    );
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "pr",
            "review-loop",
            "validate",
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let shape = parse_envelope(&out.stdout)["data"]["shape"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        op["output"]["data"].as_str().unwrap().contains(&shape),
        "output contract omits emitted {shape}"
    );
    let mut invalid = input(&request(&records[1]), &rows);
    invalid["correction"]["corrected_record"] = "invalid".into();
    let file = write_findings(&stub, "invalid.json", &invalid.to_string());
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "pr",
            "review-loop",
            "validate",
            "--findings-file",
            &file,
        ],
    );
    assert_ne!(out.code, 0);
    let code = parse_envelope(&out.stdout)["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        op["validations"]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(&code)),
        "error contract omits emitted {code}"
    );
}

#[test]
fn canonical_correction_can_clear_historical_alias_stop_with_exact_attempted_proof() {
    let (mut records, rows, attempted) = fixture(true);
    let target = records[1].clone();
    let history = chain(&records);
    let state = review_state::latest_review_loop_state(&history).unwrap();
    let error = review_state::observe_review_loop(Some(state), B, &attempted).unwrap_err();
    assert_eq!(error.kind(), "review_finding_reopened");
    let stopped = review_state::record_review_loop_hard_stop(
        state,
        B,
        &attempted,
        history.tip_digest.as_deref().unwrap(),
        &error,
    )
    .unwrap();
    append(
        &mut records,
        B,
        ReviewStatePayload::ReviewLoop { state: stopped },
        None,
    );
    let stub = StubEnv::new();
    let mut body = input(&request(&target), &rows);
    body["correction"]["attempted_findings"] = serde_json::to_value(&attempted).unwrap();
    let file = write_findings(&stub, "alias-proof.json", &body.to_string());
    let script = gh_stub_with_ledger(
        &stub,
        B,
        &records
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>(),
    );
    let stub = stub.gh_stub(&script);
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
            B,
            "--expected-state",
            records.last().unwrap().record_digest.as_str(),
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], true, "{envelope}");
    assert_eq!(envelope["data"]["would_append"], true);
    let log = gh_args_log(&stub);
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(!log.contains(marker), "unexpected mutation: {log}");
    }
}
