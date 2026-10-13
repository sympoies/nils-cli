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
    let same_assignment = chain(&records);
    assert!(
        !correction::apply(
            &same_assignment,
            B,
            same_assignment.tip_digest.as_deref(),
            &request,
            &rows
        )
        .unwrap()
        .changed
    );
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
    let ordinary_file = write_findings(
        &stub,
        "ordinary.json",
        &serde_json::to_string(
            &rows
                .iter()
                .filter(|r| r.status == ReviewFindingStatus::Open)
                .collect::<Vec<_>>(),
        )
        .unwrap(),
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
            B,
            "--expected-state",
            history.tip_digest.as_deref().unwrap(),
            "--findings-file",
            &ordinary_file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["preflight_ok"], true, "{}", out.stdout);
    assert_eq!(data["data"]["would_append"], true, "{}", out.stdout);
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

fn alias_stop() -> (
    Vec<ReviewStateRecord>,
    CorrectionRequest,
    Vec<ReviewFindingObservation>,
) {
    let (mut records, rows, attempted) = fixture(true);
    let mut request = request(&records[1]);
    request.attempted_findings = Some(attempted.clone());
    let history = chain(&records);
    let state = review_state::latest_review_loop_state(&history).unwrap();
    let error = review_state::observe_review_loop(Some(state), B, &attempted).unwrap_err();
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
    (records, request, rows)
}

#[test]
fn alias_proof_receipt_replays_once_without_changing_history_or_budgets() {
    let (mut records, mut request, rows) = alias_stop();
    let old_markers: Vec<_> = records.iter().map(|r| r.marker().unwrap()).collect();
    let history = chain(&records);
    let before = review_state::latest_review_loop_state(&history).unwrap();
    // Input order is immaterial; the receipt carries sorted canonical proof.
    request.attempted_findings.as_mut().unwrap().reverse();
    let corrected =
        correction::apply(&history, B, history.tip_digest.as_deref(), &request, &rows).unwrap();
    assert!(corrected.state.hard_stop.is_none());
    assert_eq!(corrected.state.round, before.round);
    assert_eq!(
        corrected.state.no_progress_rounds,
        before.no_progress_rounds
    );
    assert_eq!(corrected.state.budget, before.budget);
    assert_eq!(corrected.state.extensions, before.extensions);
    for (key, finding) in &corrected.state.findings {
        let original = &before.findings[key];
        assert_eq!(finding.reopen_count, original.reopen_count);
        assert_eq!(finding.seen_count, original.seen_count);
        assert_eq!(finding.last_seen_head, original.last_seen_head);
    }
    let proof = corrected
        .correction
        .as_ref()
        .unwrap()
        .attempted_findings
        .as_ref()
        .unwrap();
    assert_eq!(proof[0].fingerprint, "correctness:inventory:alias-f1");
    let expected = corrected.state.clone();
    append(&mut records, B, corrected.into_payload(), None);
    let corrected_chain = chain(&records);
    assert_eq!(
        review_state::latest_review_loop_state(&corrected_chain),
        Some(&expected)
    );
    assert_eq!(
        records[..3]
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>(),
        old_markers
    );
    let retry = correction::apply(
        &corrected_chain,
        B,
        corrected_chain.tip_digest.as_deref(),
        &request,
        &rows,
    )
    .unwrap();
    assert!(!retry.changed);
    // Ordinary same-head inventory can establish a new interval without
    // repeating a correction: its payload is an ordinary snapshot.
    let ordinary_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.status == ReviewFindingStatus::Open)
        .cloned()
        .collect();
    let ordinary = review_state::observe_review_loop(Some(&expected), B, &ordinary_rows).unwrap();
    assert!(matches!(
        ordinary.into_payload(),
        ReviewStatePayload::ReviewLoop { .. }
    ));
}

#[test]
fn alias_proof_cannot_clear_a_different_inventory_or_forge_readback() {
    let (records, request, rows) = alias_stop();
    let history = chain(&records);
    let reject = |candidate: &CorrectionRequest, current: &[ReviewFindingObservation]| {
        assert_eq!(
            correction::apply(
                &history,
                B,
                history.tip_digest.as_deref(),
                candidate,
                current
            )
            .unwrap_err()
            .kind(),
            "review_correction_invalid"
        );
    };
    let mut missing = request.clone();
    missing.attempted_findings = None;
    reject(&missing, &rows);
    for variation in 0..6 {
        let mut altered = request.clone();
        let proof = altered.attempted_findings.as_mut().unwrap();
        match variation {
            0 => proof[0].fingerprint = "correctness:inventory:other-alias".into(),
            1 => proof[0].blocking = false,
            2 => proof[0].threads.push("other-thread".into()),
            3 => proof[0].status = ReviewFindingStatus::Fixed,
            4 => proof[0].root_cause_fingerprint = Some("correctness:inventory:other-root".into()),
            _ => {
                proof.pop();
            }
        }
        reject(&altered, &rows);
    }
    let mut changed_current = rows.clone();
    changed_current[0].blocking = false;
    reject(&request, &changed_current);
    let corrected =
        correction::apply(&history, B, history.tip_digest.as_deref(), &request, &rows).unwrap();
    for variation in 0..3 {
        let mut payload = corrected.clone().into_payload();
        let ReviewStatePayload::ReviewLoopCorrection { state, correction } = &mut payload else {
            unreachable!()
        };
        match variation {
            0 => {
                correction.attempted_findings.as_mut().unwrap()[0].fingerprint =
                    "correctness:inventory:other-alias".into()
            }
            1 => correction.attempted_findings.as_mut().unwrap().reverse(),
            _ => state.budget.max_auto_reopens_per_fingerprint = 1,
        }
        let mut forged = records.clone();
        append(&mut forged, B, payload, None);
        assert_eq!(
            review_state::parse_chain(
                forged
                    .iter()
                    .map(|r| r.marker().unwrap())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(String::as_str),
                "acme/widgets",
                7
            )
            .unwrap_err()
            .kind(),
            "review_correction_invalid"
        );
    }
    let (without_stop, current, attempted) = fixture(false);
    let history = chain(&without_stop);
    let mut no_stop_request = self::request(&without_stop[1]);
    no_stop_request.attempted_findings = Some(attempted);
    assert_eq!(
        correction::apply(
            &history,
            B,
            history.tip_digest.as_deref(),
            &no_stop_request,
            &current
        )
        .unwrap_err()
        .kind(),
        "review_correction_invalid"
    );
}

#[test]
fn live_alias_correction_reads_back_normalized_proof_and_retries_without_post() {
    let (records, request, rows) = alias_stop();
    let history = chain(&records);
    let transition =
        correction::apply(&history, B, history.tip_digest.as_deref(), &request, &rows).unwrap();
    let expected = transition.state.clone();
    let mut after_records = records.clone();
    append(&mut after_records, B, transition.into_payload(), None);
    let corrected = after_records.last().unwrap();
    let page = |records: &[ReviewStateRecord]| {
        serde_json::json!({"data": {
            "viewer":{"login":"forge-bot"},
            "repository":{"pullRequest":{"comments":{
                "nodes":records.iter().map(|r| serde_json::json!({
                    "author":{"login":"forge-bot"},"authorAssociation":"OWNER",
                    "body":r.marker().unwrap(),"createdAt":"2026-07-20T12:00:00Z"
                })).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false,"endCursor":null}
            }}}
        }})
        .to_string()
    };
    let stub = StubEnv::new();
    let posts = stub.tempdir.path().join("posted");
    let script = gh_stub_with_head(&stub, B).replace(
        "esac\necho \"stub: unscripted gh args: $*\" >&2",
        &format!(
            r#"  "api graphql")
    if [ -f "{posts}" ]; then
      cat <<'AFTER'
{after}
AFTER
    else
      cat <<'BEFORE'
{before}
BEFORE
    fi
    exit 0
    ;;
esac
case "$1" in
  api)
    echo post >> "{posts}"
    echo "https://github.com/acme/widgets/pull/7#issuecomment-9"
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#,
            posts = posts.display(),
            before = page(&records),
            after = page(&after_records)
        ),
    );
    let mut body = input(&request, &rows);
    for row in body["correction"]["attempted_findings"]
        .as_array_mut()
        .unwrap()
    {
        let row = row.as_object_mut().unwrap();
        let fingerprint = row.remove("fingerprint").unwrap();
        row.insert("lifecycle_fingerprint".into(), fingerprint);
        let status = row.remove("status").unwrap();
        row.insert("disposition".into(), status);
    }
    let file = write_findings(&stub, "alias-live.json", &body.to_string());
    let stub = stub.gh_stub(&script);
    for (tip, appended) in [
        (history.tip_digest.as_deref().unwrap(), true),
        (corrected.record_digest.as_str(), false),
    ] {
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
                tip,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(out.code, 0, "{}", out.stdout);
        let data = parse_envelope(&out.stdout);
        assert_eq!(data["data"]["appended"], appended);
        assert_eq!(
            data["data"]["state"],
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(data["data"]["state_tip_digest"], corrected.record_digest);
    }
    assert_eq!(fs::read_to_string(posts).unwrap(), "post\n");
}
