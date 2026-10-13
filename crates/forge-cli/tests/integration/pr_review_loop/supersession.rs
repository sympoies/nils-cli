use super::*;
use forge_cli::ops::review_state::{
    ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
    apply_review_loop_extension, observe_review_loop, record_review_loop_hard_stop,
};
use pretty_assertions::assert_eq;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn stopped_fixture() -> (Vec<ReviewStateRecord>, Vec<ReviewFindingObservation>) {
    let findings = vec![ReviewFindingObservation {
        fingerprint: "correctness:limit:example".into(),
        root_cause_fingerprint: None,
        blocking: true,
        status: ReviewFindingStatus::Open,
        threads: vec!["thread-1".into()],
    }];
    let mut state =
        observe_review_loop(None, "1111111111111111111111111111111111111111", &findings)
            .unwrap()
            .state;
    state.round = state.budget.max_repair_rounds;
    let genesis = ReviewStateRecord::new(
        "acme/widgets",
        7,
        state.head_sha.clone(),
        0,
        None,
        ReviewStatePayload::ReviewLoop {
            state: state.clone(),
        },
    )
    .unwrap();
    let error = observe_review_loop(Some(&state), A, &findings).unwrap_err();
    assert_eq!(error.kind(), "review_round_limit_exceeded");
    let stopped =
        record_review_loop_hard_stop(&state, A, &findings, &genesis.record_digest, &error).unwrap();
    let stop_record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        A,
        1,
        Some(genesis.record_digest.clone()),
        ReviewStatePayload::ReviewLoop {
            state: stopped.clone(),
        },
    )
    .unwrap();
    let stop = stopped.hard_stop.as_ref().unwrap();
    let extended = apply_review_loop_extension(
        &stopped,
        stop.proposal_digest.clone(),
        "https://github.com/acme/widgets/pull/7#issuecomment-10".into(),
        &stop.code,
        &stop.budget_field,
        stop.increment,
    )
    .unwrap();
    let extension = ReviewStateRecord::new(
        "acme/widgets",
        7,
        A,
        2,
        Some(stop_record.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state: extended },
    )
    .unwrap();
    (vec![genesis, stop_record, extension], findings)
}

fn compare_stub(stub: &StubEnv, records: &[ReviewStateRecord]) -> String {
    gh_stub_with_ledger(stub, B, &records.iter().map(|r| r.marker().unwrap()).collect::<Vec<_>>()).replace(
        "esac\necho \"stub: unscripted gh args: $*\" >&2",
        &format!(r#"  "api repos/acme/widgets/compare/{A}...{B}?per_page=100&page=1")
    cat <<'JSON'
{{"status":"ahead","ahead_by":1,"behind_by":0,"total_commits":1,"base_commit":{{"sha":"{A}"}},"merge_base_commit":{{"sha":"{A}"}},"commits":[{{"sha":"{B}","commit":{{"verification":{{"verified":true,"reason":"valid"}}}}}}]}}
JSON
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#))
}

#[test]
fn ordinary_resume_cannot_reuse_a_approval_at_fast_forward_b() {
    let (records, findings) = stopped_fixture();
    let ReviewStatePayload::ReviewLoop { state } = &records[2].payload else {
        unreachable!()
    };
    let error = observe_review_loop(Some(state), B, &findings).unwrap_err();
    assert_eq!(error.kind(), "review_state_conflict");
    assert!(state.hard_stop.as_ref().unwrap().extension_applied);
}

#[test]
fn dry_run_records_a_new_stop_for_signed_fast_forward_after_a_extension() {
    let stub = StubEnv::new();
    let (records, findings) = stopped_fixture();
    let body = serde_json::json!({
        "schema_version":"forge-cli.review-loop-supersession.v1",
        "supersession":{"superseded_record":records[2].record_digest,"reason":"signed-fast-forward"},
        "findings":findings
    });
    let path = write_findings(&stub, "supersession.json", &body.to_string());
    let script = compare_stub(&stub, &records);
    let stub = stub.gh_stub(&script);
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
            B,
            "--expected-state",
            &records[2].record_digest,
            "--findings-file",
            &path,
        ],
    );
    assert_eq!(output.code, 0, "{} {}", output.stderr, output.stdout);
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], true, "{envelope}");
    assert_eq!(envelope["data"]["would_append"], true);
    assert!(
        envelope["data"]["planned_comment"]["visible_metadata"]
            .as_str()
            .unwrap()
            .contains("supersed")
    );
    let log = gh_args_log(&stub);
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(!log.contains(marker), "unexpected mutation: {log}");
    }
}
