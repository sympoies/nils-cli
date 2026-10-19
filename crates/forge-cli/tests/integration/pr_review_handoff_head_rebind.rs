//! Regressions for reviewer surrender when the provider head has moved.

use super::pr_review_handoff::{fixture, handoff, records, writable_ledger};
use super::support::{parse_envelope, run_forge_cli};
use pretty_assertions::{assert_eq, assert_ne};

const ASSIGNED_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CURRENT_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn surrender_after_unobserved_head_advance_records_current_provider_head() {
    let seeded = records(handoff(None), None);
    let tip = seeded.last().unwrap().record_digest.clone();
    let stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
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
            "surrender",
            "7",
            "--expected-head",
            ASSIGNED_HEAD,
            "--expected-state",
            &tip,
        ],
    );

    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["status"], "surrendered");
    assert_eq!(data["head_sha"], CURRENT_HEAD);
    assert_eq!(data["handoff"]["surrendered"], true);
    assert_eq!(data["handoff"]["assigned_head"], CURRENT_HEAD);
    assert_ne!(data["state_tip_digest"], tip);
}

#[test]
fn surrender_at_current_head_keeps_existing_behavior() {
    let seeded = records(handoff(None), None);
    let tip = seeded.last().unwrap().record_digest.clone();
    let stub = writable_ledger(fixture(ASSIGNED_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
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
            "surrender",
            "7",
            "--expected-head",
            ASSIGNED_HEAD,
            "--expected-state",
            &tip,
        ],
    );

    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["status"], "surrendered");
    assert_eq!(data["head_sha"], ASSIGNED_HEAD);
    assert_eq!(data["handoff"]["assigned_head"], ASSIGNED_HEAD);
    assert_ne!(data["state_tip_digest"], tip);
}

#[test]
fn surrender_after_unobserved_head_advance_rejects_stale_tip() {
    let seeded = records(handoff(None), None);
    let stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
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
            "surrender",
            "7",
            "--expected-head",
            ASSIGNED_HEAD,
            "--expected-state",
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ],
    );

    assert_eq!(out.code, 65);
    let error = parse_envelope(&out.stdout)["error"].clone();
    assert_eq!(error["code"], "review_state_conflict");
    assert_eq!(
        error["message"],
        "ledger tip differs from the explicit handoff tip"
    );
}

#[test]
fn surrender_after_observed_head_advance_requires_current_head() {
    let seeded = records(handoff(None), Some(ASSIGNED_HEAD));
    let tip = seeded.last().unwrap().record_digest.clone();
    let stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
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
            "surrender",
            "7",
            "--expected-head",
            ASSIGNED_HEAD,
            "--expected-state",
            &tip,
        ],
    );

    assert_eq!(out.code, 65);
    let error = parse_envelope(&out.stdout)["error"].clone();
    assert_eq!(error["code"], "review_state_conflict");
    assert_eq!(
        error["message"],
        "provider head differs from the handoff head"
    );
}
