use super::*;
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};

const SCHEMA: &str = "forge-cli.review-loop-acceptance.v1";

fn acceptance_input(
    records: &[forge_cli::ops::review_state::ReviewStateRecord],
    findings: &[forge_cli::ops::review_state::ReviewFindingObservation],
) -> (serde_json::Value, String) {
    let mut accepted = findings.to_vec();
    for row in &mut accepted {
        row.status = forge_cli::ops::review_state::ReviewFindingStatus::Accepted;
    }
    let canonical = |rows: &[forge_cli::ops::review_state::ReviewFindingObservation]| {
        rows.iter()
            .map(|row| {
                let mut row = row.clone();
                if row.status != forge_cli::ops::review_state::ReviewFindingStatus::Open {
                    row.blocking = false;
                }
                (row.fingerprint.clone(), row)
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let bytes = serde_json::to_vec(&(
        SCHEMA,
        &records.last().unwrap().record_digest,
        supersession::A,
        canonical(findings),
        canonical(&accepted),
    ))
    .unwrap();
    let proposal = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    (
        serde_json::json!({
            "schema_version": SCHEMA,
            "acceptance": {
                "stopped_record": records.last().unwrap().record_digest,
                "attempted_findings": findings,
                "approval_comment": 21
            },
            "findings": accepted
        }),
        format!("<!-- forge-cli:review-loop-acceptance:v1 proposal={proposal} -->"),
    )
}

#[test]
fn approved_terminal_acceptance_passes_preflight_without_provider_writes() {
    let stub = StubEnv::new();
    let (records, findings) = supersession::stopped_fixture();
    let (input, marker) = acceptance_input(&records, &findings);
    let approval = serde_json::json!({
        "html_url":"https://github.com/acme/widgets/pull/7#issuecomment-21",
        "issue_url":"https://api.github.com/repos/acme/widgets/issues/7",
        "user":{"login":"maintainer"},"author_association":"MEMBER",
        "created_at":"2026-07-20T12:00:01Z","body":marker
    });
    let script = approval_script(&stub, &records, &approval, None);
    let file = write_findings(&stub, "acceptance.json", &input.to_string());
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
            supersession::A,
            "--expected-state",
            &records.last().unwrap().record_digest,
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], true, "{envelope}");
    assert_eq!(envelope["data"]["would_append"], true);
    let log = gh_args_log(&stub);
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(!log.contains(marker), "unexpected mutation: {log}");
    }
}

use forge_cli::ops::review_state::{self, ReviewStatePayload, ReviewStateRecord};

fn page(records: &[ReviewStateRecord]) -> String {
    serde_json::json!({"data":{"viewer":{"login":"forge-bot"},"repository":{"pullRequest":{"comments":{
        "nodes": records.iter().enumerate().map(|(i, r)| serde_json::json!({
            "author":{"login":"forge-bot"},"authorAssociation":"OWNER","body":r.marker().unwrap(),
            "createdAt": if i < 3 {"2026-07-20T12:00:00Z"} else {"2026-07-20T12:00:02Z"}
        })).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}}}).to_string()
}

fn approval(marker: &str) -> serde_json::Value {
    serde_json::json!({
        "html_url":"https://github.com/acme/widgets/pull/7#issuecomment-21",
        "issue_url":"https://api.github.com/repos/acme/widgets/issues/7",
        "user":{"login":"maintainer"},"author_association":"MEMBER",
        "created_at":"2026-07-20T12:00:01Z","body":marker
    })
}

fn approval_script(
    stub: &StubEnv,
    records: &[ReviewStateRecord],
    approval: &serde_json::Value,
    appended: Option<&ReviewStateRecord>,
) -> String {
    let before = page(records);
    let mut rows = records.to_vec();
    rows.extend(appended.cloned());
    let after = page(&rows);
    let posts = stub.tempdir.path().join("posted");
    gh_stub_with_head(stub, supersession::A).replace(
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
case "$*" in
  *"--method POST"*)
    echo post >> "{posts}"
    echo "https://github.com/acme/widgets/pull/7#issuecomment-22"
    exit 0
    ;;
esac
echo "stub: unscripted gh args: $*" >&2"#,
            posts = posts.display()
        ),
    )
}

fn observe(stub: &StubEnv, input: &serde_json::Value, tip: &str, dry: bool) -> serde_json::Value {
    let path = write_findings(stub, "acceptance.json", &input.to_string());
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry {
        args.push("--dry-run");
    }
    args.extend([
        "pr",
        "review-loop",
        "observe",
        "7",
        "--expected-head",
        supersession::A,
        "--expected-state",
        tip,
        "--findings-file",
        &path,
    ]);
    let output = run_forge_cli(stub, &args);
    parse_envelope(&output.stdout)
}

#[test]
fn acceptance_refuses_unauthorized_stale_or_mismatched_approval_before_writes() {
    for case in [
        "outsider",
        "stale",
        "wrong-pr",
        "wrong-proposal",
        "missing",
        "extension-marker",
        "missing-metadata",
    ] {
        let stub = StubEnv::new();
        let (records, findings) = supersession::stopped_fixture();
        let (mut input, marker) = acceptance_input(&records, &findings);
        let mut approval = approval(&marker);
        match case {
            "outsider" => approval["author_association"] = "NONE".into(),
            "stale" => approval["created_at"] = "2026-07-20T12:00:00Z".into(),
            "wrong-pr" => approval["issue_url"] = "https://api.github.com/repos/acme/widgets/issues/8".into(),
            "wrong-proposal" => approval["body"] = marker.replace("sha256:", "sha256:0").into(),
            "missing" => {input["acceptance"].as_object_mut().unwrap().remove("approval_comment");},
            "extension-marker" => approval["body"] = "<!-- forge-cli:review-loop-extension:v1 proposal=sha256:example field=max_repair_rounds increment=1 -->".into(),
            "missing-metadata" => {approval.as_object_mut().unwrap().remove("user");},
            _ => unreachable!(),
        }
        let script = approval_script(&stub, &records, &approval, None);
        let stub = stub.gh_stub(&script);
        for dry in [true, false] {
            let out = observe(&stub, &input, &records.last().unwrap().record_digest, dry);
            if dry {
                assert_eq!(out["data"]["preflight_ok"], false, "{case}: {out}");
                assert!(
                    out.to_string()
                        .contains("review_acceptance_approval_invalid"),
                    "{case}: {out}"
                );
            } else {
                assert_eq!(
                    out["error"]["code"], "review_acceptance_approval_invalid",
                    "{case}: {out}"
                );
                assert_eq!(out["error"]["details"]["retryable"], false);
                assert_eq!(
                    out["error"]["details"]["next_action"],
                    "obtain-maintainer-approval"
                );
                assert_eq!(
                    out["error"]["details"]["recovery"]["approval_marker"],
                    marker
                );
            }
            if case == "missing" {
                assert!(
                    out.to_string().contains(&marker),
                    "proposal must be discoverable: {out}"
                );
            }
        }
        assert!(!stub.tempdir.path().join("posted").exists(), "{case}");
    }
}

fn accepted_record(records: &[ReviewStateRecord], input: &serde_json::Value) -> ReviewStateRecord {
    let request = serde_json::from_value(input["acceptance"].clone()).unwrap();
    let findings: Vec<review_state::ReviewFindingObservation> =
        serde_json::from_value(input["findings"].clone()).unwrap();
    let chain = supersession::chain(records);
    let transition = review_state::acceptance::apply(
        &chain,
        supersession::A,
        chain.tip_digest.as_deref(),
        &request,
        &findings,
        "https://github.com/acme/widgets/pull/7#issuecomment-21".into(),
    )
    .unwrap();
    ReviewStateRecord::new(
        "acme/widgets",
        7,
        supersession::A,
        records.len() as u64,
        chain.tip_digest,
        transition.into_payload(),
    )
    .unwrap()
}

#[test]
fn live_acceptance_is_idempotent_inspectable_and_keeps_original_records() {
    let stub = StubEnv::new();
    let (records, findings) = supersession::stopped_fixture();
    let (input, marker) = acceptance_input(&records, &findings);
    let accepted = accepted_record(&records, &input);
    let script = approval_script(&stub, &records, &approval(&marker), Some(&accepted));
    let stub = stub.gh_stub(&script);
    for (tip, appended) in [
        (&records.last().unwrap().record_digest, true),
        (&accepted.record_digest, false),
    ] {
        let out = observe(&stub, &input, tip, false);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["appended"], appended);
        assert_eq!(
            out["data"]["accepted_risks"][0]["accepted_fingerprints"],
            serde_json::json!([findings[0].fingerprint])
        );
    }
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
            "inspect",
            "7",
        ],
    );
    let out = parse_envelope(&out.stdout);
    assert_eq!(out["ok"], true, "{out}");
    let receipt = &out["data"]["accepted_risks"][0];
    assert_eq!(
        receipt["original_stop"]["code"],
        "review_round_limit_exceeded"
    );
    assert_eq!(
        receipt["request"]["attempted_findings"],
        input["acceptance"]["attempted_findings"]
    );
    assert_eq!(
        receipt["approval_reference"],
        "https://github.com/acme/widgets/pull/7#issuecomment-21"
    );
    assert_eq!(
        fs::read_to_string(stub.tempdir.path().join("posted")).unwrap(),
        "post\n"
    );
    let mut rows = records.clone();
    rows.push(accepted);
    let parsed = supersession::chain(&rows);
    assert_eq!(&parsed.records[..records.len()], records);
    let before = review_state::latest_review_loop_state(&supersession::chain(&records))
        .unwrap()
        .clone();
    let after = review_state::latest_review_loop_state(&parsed).unwrap();
    assert_eq!(after.round, before.round);
    assert_eq!(after.no_progress_rounds, before.no_progress_rounds);
    assert_eq!(after.budget, before.budget);
    assert_eq!(after.extensions, before.extensions);
    assert_eq!(
        after.findings[&findings[0].fingerprint].seen_count,
        before.findings[&findings[0].fingerprint].seen_count
    );
    assert!(after.hard_stop.is_none());
}

struct LedgerRunner(String);
impl forge_cli::backend::BackendRunner for LedgerRunner {
    fn run(
        &self,
        call: &forge_cli::backend::BackendCall,
    ) -> Result<forge_cli::backend::BackendSuccess, forge_cli::error::ForgeError> {
        assert_eq!(call.argv[0], "api");
        assert_eq!(call.argv[1], "graphql");
        Ok(forge_cli::backend::BackendSuccess {
            stdout: self.0.clone(),
            stderr: String::new(),
        })
    }
}

#[test]
fn accepted_risk_converges_without_extension_or_another_repair_round() {
    for already_extended in [false, true] {
        let (mut records, findings) = supersession::stopped_fixture();
        if !already_extended {
            records.pop();
        }
        let ctx = forge_cli::provider::ProviderContext {
            provider: forge_cli::provider::Provider::GitHub,
            host: "github.com".into(),
            source: forge_cli::provider::DetectionSource::Flag,
            repo: Some("acme/widgets".into()),
        };
        let gate = |rows: &[ReviewStateRecord]| {
            forge_cli::ops::pr_review_loop::ensure_merge_ready(
                &LedgerRunner(page(rows)),
                &ctx,
                "https://github.com/acme/widgets/pull/7",
                7,
                supersession::A,
                true,
            )
        };
        assert_eq!(
            gate(&records).unwrap_err().kind(),
            "review_round_limit_exceeded"
        );
        let (input, _) = acceptance_input(&records, &findings);
        let accepted = accepted_record(&records, &input);
        records.push(accepted);
        let converged = gate(&records).unwrap().unwrap();
        assert_eq!(converged.round, 5);
        assert!(converged.open_blocking_findings.is_empty());
    }
}

#[test]
fn acceptance_refuses_changed_proof_inventory_or_rehashed_budget_tampering() {
    let (records, findings) = supersession::stopped_fixture();
    let (input, _) = acceptance_input(&records, &findings);
    let chain = supersession::chain(&records);
    for case in ["head", "proof", "status", "omission", "tip", "record"] {
        let mut changed = input.clone();
        match case {
            "proof" => {
                changed["acceptance"]["attempted_findings"][0]["threads"] = serde_json::json!([])
            }
            "status" => changed["findings"][0]["status"] = "fixed".into(),
            "omission" => changed["findings"] = serde_json::json!([]),
            "record" => {
                changed["acceptance"]["stopped_record"] = records[0].record_digest.clone().into()
            }
            _ => (),
        }
        let request = serde_json::from_value(changed["acceptance"].clone()).unwrap();
        let rows = serde_json::from_value::<Vec<review_state::ReviewFindingObservation>>(
            changed["findings"].clone(),
        )
        .unwrap();
        let result = review_state::acceptance::apply(
            &chain,
            if case == "head" {
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            } else {
                supersession::A
            },
            if case == "tip" {
                None
            } else {
                chain.tip_digest.as_deref()
            },
            &request,
            &rows,
            "https://github.com/acme/widgets/pull/7#issuecomment-21".into(),
        );
        assert_eq!(
            result.unwrap_err().kind(),
            "review_acceptance_invalid",
            "{case}"
        );
    }
    for tamper in ["round", "budget", "fingerprint", "stop"] {
        let mut record = accepted_record(&records, &input);
        let ReviewStatePayload::ReviewLoopAcceptance { state, acceptance } = &mut record.payload
        else {
            unreachable!()
        };
        match tamper {
            "round" => state.round = 0,
            "budget" => state.budget.max_repair_rounds += 1,
            "fingerprint" => acceptance.accepted_fingerprints.clear(),
            "stop" => acceptance.original_stop.observation_digest = "sha256:other".into(),
            _ => unreachable!(),
        }
        record.record_digest = record.compute_digest().unwrap();
        let mut rows = records.clone();
        rows.push(record);
        let markers: Vec<_> = rows.iter().map(|r| r.marker().unwrap()).collect();
        assert_eq!(
            review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7)
                .unwrap_err()
                .kind(),
            "review_acceptance_invalid",
            "{tamper}"
        );
    }
}

#[test]
fn acceptance_requires_explicit_tip_and_strict_envelope_before_writes() {
    for case in ["auto-state", "stale-tip", "unknown-field", "unknown-schema"] {
        let stub = StubEnv::new();
        let (records, findings) = supersession::stopped_fixture();
        let (mut input, marker) = acceptance_input(&records, &findings);
        if case == "unknown-field" {
            input["acceptance"]["unapproved"] = true.into();
        }
        if case == "unknown-schema" {
            input["schema_version"] = "forge-cli.review-loop-acceptance.v2".into();
        }
        let script = approval_script(&stub, &records, &approval(&marker), None);
        let file = write_findings(&stub, "acceptance.json", &input.to_string());
        let stub = stub.gh_stub(&script);
        let mut args = vec![
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
            supersession::A,
            "--findings-file",
            &file,
        ];
        if case == "auto-state" {
            args.push("--auto-state");
        } else {
            args.extend([
                "--expected-state",
                if case == "stale-tip" {
                    &records[0].record_digest
                } else {
                    &records.last().unwrap().record_digest
                },
            ]);
        }
        let out = run_forge_cli(&stub, &args);
        assert_eq!(out.code, 65, "{case}: {} {}", out.stdout, out.stderr);
        let out = parse_envelope(&out.stdout);
        assert_eq!(
            out["error"]["code"],
            if case == "stale-tip" {
                "review_state_conflict"
            } else {
                "review_acceptance_invalid"
            },
            "{case}: {out}"
        );
        if case != "stale-tip" {
            assert_eq!(out["error"]["details"]["retryable"], false);
            assert_eq!(
                out["error"]["details"]["next_action"],
                "inspect-stopped-inventory"
            );
        }
        assert!(!stub.tempdir.path().join("posted").exists());
    }
}

#[test]
fn canonical_acceptance_retry_cannot_reestablish_a_reassigned_review_interval() {
    let (mut records, findings) = supersession::stopped_fixture();
    let (input, _) = acceptance_input(&records, &findings);
    let mut owner = super::super::pr_review_handoff::handoff(None);
    let append = |rows: &mut Vec<ReviewStateRecord>,
                  owner: &forge_cli::ops::pr_review_handoff::ReviewHandoff| {
        let row = ReviewStateRecord::new(
            "acme/widgets",
            7,
            supersession::A,
            rows.len() as u64,
            rows.last().map(|r| r.record_digest.clone()),
            ReviewStatePayload::ReviewHandoff {
                handoff: owner.clone(),
            },
        )
        .unwrap();
        rows.push(
            row.with_assignment_generation(Some(owner.assignment_generation))
                .unwrap(),
        );
    };
    append(&mut records, &owner);
    let accepted = accepted_record(&records, &input)
        .with_assignment_generation(Some(1))
        .unwrap();
    records.push(accepted);
    owner.surrendered = true;
    append(&mut records, &owner);
    owner.surrendered = false;
    owner.assignment_generation = 2;
    append(&mut records, &owner);
    let chain = supersession::chain(&records);
    let request = serde_json::from_value(input["acceptance"].clone()).unwrap();
    let findings: Vec<review_state::ReviewFindingObservation> =
        serde_json::from_value(input["findings"].clone()).unwrap();
    let error = review_state::acceptance::apply(
        &chain,
        supersession::A,
        chain.tip_digest.as_deref(),
        &request,
        &findings,
        "https://github.com/acme/widgets/pull/7#issuecomment-21".into(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "review_acceptance_invalid");
    assert!(error.message().contains("earlier-assignment"));
}
