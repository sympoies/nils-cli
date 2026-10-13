use super::*;
use forge_cli::ops::review_state::{
    ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
    observe_review_loop,
};
use pretty_assertions::assert_eq;

fn ledger_fixture() -> (Vec<ReviewStateRecord>, Vec<ReviewFindingObservation>) {
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
    (vec![genesis, erroneous_record], corrected)
}

#[test]
fn dry_run_accepts_chain_bound_omission_correction_without_reopen_budget() {
    let stub = StubEnv::new();
    let (records, corrected) = ledger_fixture();
    let erroneous_record = &records[1];
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
        &records
            .iter()
            .map(|record| record.marker().unwrap())
            .collect::<Vec<_>>(),
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

#[test]
fn live_correction_appends_history_once_and_keeps_zero_reopen_budget() {
    use forge_cli::ops::review_state::correction::{CorrectionReason, CorrectionReceipt};
    let stub = StubEnv::new();
    let (records, findings) = ledger_fixture();
    let target = &records[1];
    let ReviewStatePayload::ReviewLoop { state } = &target.payload else {
        unreachable!()
    };
    let mut expected = state.clone();
    let restored: Vec<_> = (1..=4)
        .map(|i| format!("correctness:inventory:f{i}"))
        .collect();
    for key in &restored {
        expected.findings.get_mut(key).unwrap().status = ReviewFindingStatus::Open;
    }
    let corrected = ReviewStateRecord::new(
        "acme/widgets",
        7,
        "head-2",
        2,
        Some(target.record_digest.clone()),
        ReviewStatePayload::ReviewLoopCorrection {
            state: expected.clone(),
            correction: CorrectionReceipt {
                corrected_record: target.record_digest.clone(),
                reason: CorrectionReason::OmittedFindings,
                restored_findings: restored,
            },
        },
    )
    .unwrap();
    let page = |records: &[ReviewStateRecord]| {
        serde_json::json!({"data": {
            "viewer": {"login": "forge-bot"},
            "repository": {"pullRequest": {"comments": {
                "nodes": records.iter().map(|record| serde_json::json!({
                    "author": {"login":"forge-bot"}, "authorAssociation":"OWNER",
                    "body": record.marker().unwrap(), "createdAt":"2026-07-20T12:00:00Z"
                })).collect::<Vec<_>>(), "pageInfo":{"hasNextPage":false,"endCursor":null}
            }}}
        }})
        .to_string()
    };
    let before = page(&records);
    let mut after_records = records.clone();
    after_records.push(corrected.clone());
    let after = page(&after_records);
    let posts = stub.tempdir.path().join("posted");
    let script = gh_stub_with_head(&stub, "head-2").replace(
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
            posts = posts.display()
        ),
    );
    let file = write_findings(
        &stub,
        "correction.json",
        &serde_json::json!({
            "schema_version":"forge-cli.review-loop-correction.v1",
            "correction":{"corrected_record":target.record_digest,"reason":"omitted-findings"},
            "findings":findings,
        })
        .to_string(),
    );
    let stub = stub.gh_stub(&script);
    for (tip, appended) in [
        (&target.record_digest, true),
        (&corrected.record_digest, false),
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
                "head-2",
                "--expected-state",
                tip,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["appended"], appended);
        assert_eq!(envelope["data"]["generation"], 2);
        assert_eq!(
            envelope["data"]["state"],
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            envelope["data"]["state_tip_digest"],
            corrected.record_digest
        );
    }
    assert_eq!(fs::read_to_string(posts).unwrap(), "post\n");
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
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["shape"],
        "correction-envelope"
    );
}
