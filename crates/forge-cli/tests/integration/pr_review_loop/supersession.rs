use super::*;
use forge_cli::ops::review_state::{
    ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
    apply_review_loop_extension, observe_review_loop, record_review_loop_hard_stop,
};
use pretty_assertions::{assert_eq, assert_ne};

pub(super) const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub(super) fn stopped_fixture() -> (Vec<ReviewStateRecord>, Vec<ReviewFindingObservation>) {
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

pub(super) fn compare_stub(stub: &StubEnv, records: &[ReviewStateRecord]) -> String {
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

pub(super) fn chain(
    records: &[ReviewStateRecord],
) -> forge_cli::ops::review_state::ReviewStateChain {
    forge_cli::ops::review_state::parse_chain(
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

fn recover(
    records: &[ReviewStateRecord],
    findings: &[ReviewFindingObservation],
) -> forge_cli::ops::review_state::ReviewLoopTransition {
    use forge_cli::ops::review_state::supersession::{
        self, SupersessionReason, SupersessionRequest,
    };
    supersession::apply(
        &chain(records),
        B,
        Some(&records[2].record_digest),
        &SupersessionRequest {
            superseded_record: records[2].record_digest.clone(),
            reason: SupersessionReason::SignedFastForward,
        },
        findings,
        &[B.into()],
    )
    .unwrap()
}

#[test]
fn supersession_preserves_history_budgets_and_requires_new_b_approval() {
    use forge_cli::ops::review_state::parse_chain;
    let (mut records, findings) = stopped_fixture();
    let original_markers: Vec<_> = records.iter().map(|r| r.marker().unwrap()).collect();
    let ReviewStatePayload::ReviewLoop { state: old } = &records[2].payload else {
        unreachable!()
    };
    let old = old.clone();
    let transition = recover(&records, &findings);
    let recovered = transition.state.clone();
    let stop = recovered.hard_stop.as_ref().unwrap();
    assert_eq!(stop.attempted_head_sha, B);
    assert!(!stop.extension_applied);
    assert_ne!(
        stop.proposal_digest,
        old.hard_stop.as_ref().unwrap().proposal_digest
    );
    let mut without_stop = recovered.clone();
    without_stop.hard_stop = old.hard_stop.clone();
    assert_eq!(without_stop, old);
    assert_eq!(
        observe_review_loop(Some(&recovered), B, &findings)
            .unwrap_err()
            .kind(),
        "review_round_limit_exceeded"
    );
    let old_stop = old.hard_stop.as_ref().unwrap();
    assert_eq!(
        apply_review_loop_extension(
            &recovered,
            old_stop.proposal_digest.clone(),
            old.extensions[0].approval_reference.clone(),
            &old_stop.code,
            &old_stop.budget_field,
            old_stop.increment
        )
        .unwrap_err()
        .kind(),
        "review_extension_invalid"
    );
    let fresh = apply_review_loop_extension(
        &recovered,
        stop.proposal_digest.clone(),
        "https://github.com/acme/widgets/pull/7#issuecomment-11".into(),
        &stop.code,
        &stop.budget_field,
        stop.increment,
    )
    .unwrap();
    let resumed = observe_review_loop(Some(&fresh), B, &findings)
        .unwrap()
        .state;
    assert_eq!(resumed.head_sha, B);
    assert_eq!(resumed.round, old.round + 1);
    assert_eq!(resumed.extensions.len(), 2);
    assert_eq!(
        resumed.budget.max_repair_rounds,
        old.budget.max_repair_rounds + 1
    );
    assert!(resumed.hard_stop.is_none());
    let record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        B,
        3,
        Some(records[2].record_digest.clone()),
        transition.into_payload(),
    )
    .unwrap();
    records.push(record);
    let validated = chain(&records);
    assert_eq!(validated.records.len(), 4);
    assert_eq!(
        validated.records[..3]
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>(),
        original_markers
    );
    let mut forged = records[3].clone();
    if let ReviewStatePayload::ReviewLoopSupersession { state, .. } = &mut forged.payload {
        state.round = 0;
    }
    forged.record_digest = forged.compute_digest().unwrap();
    let mut markers = original_markers;
    markers.push(forged.marker().unwrap());
    assert_eq!(
        parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7)
            .unwrap_err()
            .kind(),
        "review_supersession_invalid"
    );
}

#[test]
fn supersession_rejects_stale_binding_incomplete_inventory_and_unknown_fields() {
    use forge_cli::ops::review_state::supersession::{
        self, SupersessionReason, SupersessionRequest,
    };
    let (records, findings) = stopped_fixture();
    let request = SupersessionRequest {
        superseded_record: records[2].record_digest.clone(),
        reason: SupersessionReason::SignedFastForward,
    };
    let chain = chain(&records);
    for (head, tip, rows) in [
        (B, None, findings.as_slice()),
        (
            B,
            Some(records[1].record_digest.as_str()),
            findings.as_slice(),
        ),
        (A, chain.tip_digest.as_deref(), findings.as_slice()),
        (B, chain.tip_digest.as_deref(), &[]),
    ] {
        assert_eq!(
            supersession::apply(&chain, head, tip, &request, rows, &[B.into()])
                .unwrap_err()
                .kind(),
            "review_supersession_invalid"
        );
    }
    let stub = StubEnv::new();
    let file = write_findings(&stub, "unknown.json", &serde_json::json!({"schema_version":"forge-cli.review-loop-supersession.v1", "supersession":{"superseded_record":request.superseded_record,"reason":"signed-fast-forward","verified":true},"findings": findings}).to_string());
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
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "review_supersession_invalid"
    );
}

#[test]
fn provider_proof_rejects_divergence_unsigned_and_truncated_results_without_writes() {
    for (old, new) in [
        ("\"status\":\"ahead\"", "\"status\":\"diverged\""),
        ("\"verified\":true", "\"verified\":false"),
        ("\"total_commits\":1", "\"total_commits\":2"),
    ] {
        let stub = StubEnv::new();
        let (records, findings) = stopped_fixture();
        let file = write_findings(&stub, "request.json", &serde_json::json!({"schema_version":"forge-cli.review-loop-supersession.v1", "supersession":{"superseded_record":records[2].record_digest,"reason":"signed-fast-forward"},"findings": findings}).to_string());
        let script = compare_stub(&stub, &records).replace(old, new);
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
                &records[2].record_digest,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["preflight_ok"], false, "{envelope}");
        assert!(envelope.to_string().contains("review_supersession_invalid"));
        let log = gh_args_log(&stub);
        for marker in PROVIDER_WRITE_MARKERS {
            assert!(!log.contains(marker), "unexpected mutation: {log}");
        }
    }
}

#[test]
fn live_supersession_appends_once_readback_keeps_original_stop_and_retry_is_noop() {
    live_supersession_with_commits(&[B.into()]);
}

pub(super) fn live_supersession_with_commits(commits: &[String]) {
    use forge_cli::ops::review_state::supersession::INPUT_SCHEMA;
    let stub = StubEnv::new();
    let (records, findings) = stopped_fixture();
    let history = chain(&records);
    let expected = forge_cli::ops::review_state::supersession::apply(
        &history,
        B,
        history.tip_digest.as_deref(),
        &forge_cli::ops::review_state::supersession::SupersessionRequest {
            superseded_record: records[2].record_digest.clone(),
            reason:
                forge_cli::ops::review_state::supersession::SupersessionReason::SignedFastForward,
        },
        &findings,
        commits,
    )
    .unwrap();
    let state = expected.state.clone();
    let appended = ReviewStateRecord::new(
        "acme/widgets",
        7,
        B,
        3,
        Some(records[2].record_digest.clone()),
        expected.into_payload(),
    )
    .unwrap();
    let page = |rows: &[ReviewStateRecord]| {
        serde_json::json!({"data":{"viewer":{"login":"forge-bot"},"repository":{"pullRequest":{"comments":{"nodes":rows.iter().map(|r|serde_json::json!({"author":{"login":"forge-bot"},"authorAssociation":"OWNER","body":r.marker().unwrap(),"createdAt":"2026-07-20T12:00:00Z"})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}).to_string()
    };
    let before = page(&records);
    let mut rows = records.clone();
    rows.push(appended.clone());
    let after = page(&rows);
    let posts = stub.tempdir.path().join("posted");
    let comparison = serde_json::json!({"status":"ahead","ahead_by":commits.len(),"behind_by":0,"total_commits":commits.len(),"base_commit":{"sha":A},"merge_base_commit":{"sha":A},"commits":commits.iter().map(|sha|serde_json::json!({"sha":sha,"commit":{"verification":{"verified":true,"reason":"valid"}}})).collect::<Vec<_>>()}).to_string();
    let script = compare_stub(&stub, &records);
    let begin = script
        .find("    cat <<'JSON'\n{\"status\":\"ahead\"")
        .unwrap()
        + "    cat <<'JSON'\n".len();
    let finish = script[begin..].find("\nJSON\n").unwrap() + begin;
    let script = format!("{}{}{}", &script[..begin], comparison, &script[finish..]);
    let start = script.find("  \"api graphql\")").unwrap();
    let end = script[start..].find("    ;;\n").unwrap() + start + "    ;;\n".len();
    let replacement = format!(
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
"#,
        posts = posts.display()
    );
    let script = format!("{}{}{}", &script[..start], replacement, &script[end..]).replace(
        "esac\necho \"stub: unscripted gh args: $*\" >&2",
        &format!(
            r#"esac
case "$1" in
  api)
    echo post >> "{posts}"
    echo "https://github.com/acme/widgets/pull/7#issuecomment-12"
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#,
            posts = posts.display()
        ),
    );
    let file = write_findings(&stub, "supersession.json", &serde_json::json!({"schema_version":INPUT_SCHEMA,"supersession":{"superseded_record":records[2].record_digest,"reason":"signed-fast-forward"},"findings":findings}).to_string());
    let stub = stub.gh_stub(&script);
    for (tip, did_append) in [
        (&records[2].record_digest, true),
        (&appended.record_digest, false),
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
        assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["appended"], did_append);
        assert_eq!(envelope["data"]["generation"], 3);
        assert_eq!(
            envelope["data"]["state"],
            serde_json::to_value(&state).unwrap()
        );
    }
    assert_eq!(fs::read_to_string(posts).unwrap(), "post\n");
}

#[test]
fn signed_fast_forward_proof_reads_complete_boundary_and_checks_intermediate_signatures() {
    for unsigned_intermediate in [false, true] {
        let stub = StubEnv::new();
        let (records, findings) = stopped_fixture();
        let commit = |sha: String, verified: bool| serde_json::json!({"sha":sha,"commit":{"verification":{"verified":verified,"reason":if verified {"valid"} else {"unsigned"}}}});
        let mut rows: Vec<_> = (1..100)
            .map(|i| commit(format!("{i:040x}"), !unsigned_intermediate || i != 50))
            .collect();
        rows.push(commit(B.into(), true));
        let page = |commits: Vec<serde_json::Value>| {
            serde_json::json!({"status":"ahead","ahead_by":100,"behind_by":0,"total_commits":100,"base_commit":{"sha":A},"merge_base_commit":{"sha":A},"commits":commits}).to_string()
        };
        let first = page(rows);
        let script = gh_stub_with_ledger(
            &stub,
            B,
            &records
                .iter()
                .map(|r| r.marker().unwrap())
                .collect::<Vec<_>>(),
        )
        .replace(
            "esac\necho \"stub: unscripted gh args: $*\" >&2",
            &format!(
                r#"  "api repos/acme/widgets/compare/{A}...{B}?per_page=100&page=1")
    cat <<'FIRST'
{first}
FIRST
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#
            ),
        );
        let file = write_findings(&stub, "paged.json", &serde_json::json!({"schema_version":"forge-cli.review-loop-supersession.v1","supersession":{"superseded_record":records[2].record_digest,"reason":"signed-fast-forward"},"findings":findings}).to_string());
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
                &records[2].record_digest,
                "--findings-file",
                &file,
            ],
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(
            envelope["data"]["preflight_ok"], !unsigned_intermediate,
            "{envelope}"
        );
        let log = gh_args_log(&stub);
        assert_eq!(log.matches("compare/").count(), 1, "{log}");
        for marker in PROVIDER_WRITE_MARKERS {
            assert!(!log.contains(marker), "unexpected mutation: {log}");
        }
    }
}

#[test]
fn extend_at_b_requires_a_new_post_stop_maintainer_approval_for_b_proposal() {
    for (old_proposal, old_timestamp) in [(true, false), (false, true), (false, false)] {
        let stub = StubEnv::new();
        let (mut records, findings) = stopped_fixture();
        let original_proposal = match &records[2].payload {
            ReviewStatePayload::ReviewLoop { state } => {
                state.hard_stop.as_ref().unwrap().proposal_digest.clone()
            }
            _ => unreachable!(),
        };
        let transition = recover(&records, &findings);
        let stop = transition.state.hard_stop.clone().unwrap();
        let extended = apply_review_loop_extension(
            &transition.state,
            stop.proposal_digest.clone(),
            "https://github.com/acme/widgets/pull/7#issuecomment-21".into(),
            &stop.code,
            &stop.budget_field,
            1,
        )
        .unwrap();
        let supersession = ReviewStateRecord::new(
            "acme/widgets",
            7,
            B,
            3,
            Some(records[2].record_digest.clone()),
            transition.into_payload(),
        )
        .unwrap();
        records.push(supersession.clone());
        let extension = ReviewStateRecord::new(
            "acme/widgets",
            7,
            B,
            4,
            Some(supersession.record_digest.clone()),
            ReviewStatePayload::ReviewLoop { state: extended },
        )
        .unwrap();
        let page = |rows: &[ReviewStateRecord]| {
            serde_json::json!({"data":{"viewer":{"login":"forge-bot"},"repository":{"pullRequest":{"comments":{"nodes":rows.iter().map(|r|serde_json::json!({"author":{"login":"forge-bot"},"authorAssociation":"OWNER","body":r.marker().unwrap(),"createdAt":"2026-07-20T12:00:01Z"})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}).to_string()
        };
        let before = page(&records);
        let mut after_records = records.clone();
        after_records.push(extension);
        let after = page(&after_records);
        let proposal = if old_proposal {
            &original_proposal
        } else {
            &stop.proposal_digest
        };
        let approval = serde_json::json!({"html_url":"https://github.com/acme/widgets/pull/7#issuecomment-21","issue_url":"https://api.github.com/repos/acme/widgets/issues/7","user":{"login":"maintainer"},"author_association":"MEMBER","created_at":if old_timestamp {"2026-07-20T12:00:00Z"} else {"2026-07-20T12:00:02Z"},"body":format!("<!-- forge-cli:review-loop-extension:v1 proposal={proposal} field={} increment=1 -->", stop.budget_field)}).to_string();
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
  "api repos/acme/widgets/issues/comments/21")
    cat <<'APPROVAL'
{approval}
APPROVAL
    exit 0
    ;;
esac
case "$1" in
  api)
    echo post >> "{posts}"
    echo "https://github.com/acme/widgets/pull/7#issuecomment-22"
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#,
                posts = posts.display()
            ),
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
                "pr",
                "review-loop",
                "extend",
                "7",
                "--expected-head",
                B,
                "--expected-state",
                &supersession.record_digest,
                "--proposal-digest",
                &stop.proposal_digest,
                "--stop-code",
                &stop.code,
                "--budget-field",
                &stop.budget_field,
                "--increment",
                "1",
                "--approval-comment",
                "21",
            ],
        );
        if old_proposal || old_timestamp {
            assert_ne!(out.code, 0, "{}", out.stdout);
            assert_eq!(
                parse_envelope(&out.stdout)["error"]["code"],
                "review_extension_approval_invalid"
            );
            assert!(!posts.exists());
        } else {
            assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
            let envelope = parse_envelope(&out.stdout);
            assert_eq!(
                envelope["data"]["state"]["hard_stop"]["extension_applied"],
                true
            );
            assert_eq!(
                envelope["data"]["state"]["extensions"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(fs::read_to_string(posts).unwrap(), "post\n");
        }
    }
}
