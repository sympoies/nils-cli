use super::*;
use pretty_assertions::assert_eq;

fn check_final(stub: &StubEnv) -> super::super::support::CmdOutput {
    run_forge_cli(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "check",
            "7",
            "--expected-head",
            HEAD,
            "--approval-policy",
            "final-approval",
        ],
    )
}

fn approved() -> Value {
    let mut report = review(HEAD, "follow-up-pass");
    report["state"] = json!("APPROVED");
    report
}

#[test]
fn comment_source_pass_reports_final_approval_pending() {
    let mut report = review(HEAD, "follow-up-pass");
    report["body"] = json!(report["body"].as_str().unwrap().replace(
        "assigned review fixture",
        "Source repair passed; final delivery remains on hold pending the required owner gate."
    ));
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![report]);
    let out = check(&stub, HEAD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["status"], "reviewed");
    assert_eq!(data["approval_policy"], "source-pass");
    assert_eq!(data["approval"]["state"], "source-passed");
    assert_eq!(data["approval"]["review_id"], "REVIEW_1");
    assert_eq!(data["approval"]["review_author"], "review-app[bot]");
    assert_eq!(data["approval"]["native_state"], "COMMENTED");
    assert_eq!(data["approval"]["reviewed_head"], HEAD);
    assert_eq!(
        data["approval"]["review_url"],
        "https://github.com/acme/widgets/pull/7#pullrequestreview-1"
    );
    assert_refusal(
        &check_final(&stub),
        65,
        "designated_final_approval_required",
    );
}

#[test]
fn final_approved_report_satisfies_strict_check() {
    let mut final_report = approved();
    final_report["id"] = json!("REVIEW_2");
    final_report["databaseId"] = json!(2);
    final_report["submittedAt"] = json!("2026-07-20T12:00:03Z");
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![review(HEAD, "pass"), final_report],
    );
    let out = check_final(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["approval_policy"], "final-approval");
    assert_eq!(data["approval"]["state"], "finally-approved");
    assert_eq!(data["approval"]["review_id"], "REVIEW_2");
    assert_eq!(data["approval"]["native_state"], "APPROVED");
}

#[test]
fn strict_check_never_reuses_superseded_or_stale_approval() {
    for (head, verdict) in [(HEAD, "blocked"), (HEAD, "pass"), (OLD, "pass")] {
        let mut approval = approved();
        approval["commit"]["oid"] = json!(head);
        let mut newer = review(HEAD, verdict);
        newer["databaseId"] = json!(2);
        newer["submittedAt"] = json!("2026-07-20T12:00:03Z");
        let stub = fixture(
            HEAD,
            &records(handoff(None), Some(HEAD)),
            vec![approval, newer],
        );
        assert_refusal(
            &check_final(&stub),
            65,
            if verdict == "blocked" {
                "awaiting_designated_review"
            } else {
                "designated_final_approval_required"
            },
        );
    }
}

#[test]
fn strict_check_ignores_proven_empty_thread_reply_after_approval() {
    let mut reply = review(HEAD, "pass");
    reply["databaseId"] = json!(2);
    reply["body"] = json!("");
    reply["submittedAt"] = json!("2026-07-20T12:00:03Z");
    let stub = fixture(
        HEAD,
        &records(handoff(None), Some(HEAD)),
        vec![approved(), reply],
    );
    let out = check_final(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["approval"]["state"], "finally-approved");
    assert_eq!(data["approval"]["review_id"], "REVIEW_1");
}

#[test]
fn assignment_can_require_final_approval() {
    let stub = fixture(HEAD, &[], vec![]);
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
            "review-handoff",
            "assign",
            "7",
            "--expected-head",
            HEAD,
            "--reviewer-session",
            REVIEWER_UUID,
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            OLD,
            "--expected-state",
            "none",
            "--approval-policy",
            "final-approval",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["handoff"]["approval_policy"],
        "final-approval"
    );
}

fn strict_owner() -> ReviewHandoff {
    let mut owner = handoff(None);
    owner.approval_policy = forge_cli::cli::ApprovalPolicy::FinalApproval;
    owner
}

#[test]
fn persisted_strict_policy_cannot_be_weakened_by_check_caller() {
    let stub = fixture(
        HEAD,
        &records(strict_owner(), Some(HEAD)),
        vec![review(HEAD, "pass")],
    );
    assert_refusal(
        &check(&stub, HEAD),
        65,
        "designated_final_approval_required",
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
            "pr",
            "review-handoff",
            "check",
            "7",
            "--expected-head",
            HEAD,
            "--approval-policy",
            "source-pass",
        ],
    );
    assert_refusal(&out, 65, "designated_final_approval_required");
    super::inspection::assert_pending(&stub);
}

#[test]
fn merge_enforces_strict_policy_even_with_explicit_review_opt_out() {
    for observed in [None, Some(HEAD)] {
        for opt_out in [false, true] {
            let stub = fixture(
                HEAD,
                &records(strict_owner(), observed),
                vec![review(HEAD, "pass")],
            );
            let mut args = vec![
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "--dry-run",
                "pr",
                "merge",
                "7",
            ];
            if opt_out {
                args.push("--review-convergence=false");
            }
            let out = run_forge_cli(&stub, &args);
            assert_refusal(
                &out,
                65,
                if observed.is_some() {
                    "designated_final_approval_required"
                } else {
                    "awaiting_designated_review"
                },
            );
        }
    }
}

#[test]
fn default_policy_preserves_old_handoff_record_digest() {
    let owner = handoff(None);
    let bytes = serde_json::to_value(&owner).unwrap();
    assert!(bytes.get("approval_policy").is_none());
    let restored: ReviewHandoff = serde_json::from_value(bytes).unwrap();
    assert_eq!(restored, owner);
    let original = records(owner, Some(HEAD));
    let serialized = serde_json::to_vec(&original).unwrap();
    let decoded: Vec<ReviewStateRecord> = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), serialized);
}

#[test]
fn merge_preserves_comment_workflow_and_accepts_strict_final_approval() {
    for (owner, report) in [
        (handoff(None), review(HEAD, "pass")),
        (strict_owner(), approved()),
    ] {
        let stub = fixture(HEAD, &records(owner, Some(HEAD)), vec![report]);
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
                "merge",
                "7",
            ],
        );
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    }
}

#[test]
fn base_refresh_preserves_strict_policy_and_cannot_silently_downgrade_it() {
    let mut owner = strict_owner();
    owner.reviewer_digest = review_state::sha256_digest(REVIEWER_UUID.as_bytes());
    let seeded = records(owner, Some(HEAD));
    let stub = fixture(HEAD, &seeded, vec![])
        .env("PROVIDER_BASE", HEAD)
        .env("AGENT_REVIEWER_SESSION", REVIEWER_UUID);
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "--dry-run",
        "pr",
        "review-handoff",
        "assign",
        "7",
        "--expected-head",
        HEAD,
        "--reviewer-session",
        REVIEWER_UUID,
        "--review-author",
        "review-app[bot]",
        "--base-sha",
        HEAD,
        "--expected-state",
        &seeded.last().unwrap().record_digest,
    ];
    let out = run_forge_cli(&stub, &args);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["handoff"]["approval_policy"],
        "final-approval"
    );
    args.extend(["--approval-policy", "source-pass"]);
    assert_refusal(&run_forge_cli(&stub, &args), 65, "review_handover_required");
}
