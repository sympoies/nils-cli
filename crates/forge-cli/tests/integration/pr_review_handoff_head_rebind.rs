//! Regressions for reviewer surrender when the provider head has moved.

use std::fs;

use forge_cli::ops::{
    pr_review_handoff::ReviewHandoff,
    review_state::{ReviewStatePayload, ReviewStateRecord},
};

use super::pr_review_handoff::{fixture, handoff, records, writable_ledger};
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::{assert_eq, assert_ne};

const ASSIGNED_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CURRENT_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const BASE_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const REVIEWER_UUID: &str = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";

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
    for observation in [None, Some(ASSIGNED_HEAD)] {
        let seeded = records(handoff(None), observation);
        let tip = seeded.last().unwrap().record_digest.clone();
        let stub = writable_ledger(fixture(ASSIGNED_HEAD, &seeded, vec![]), &seeded)
            .env("AGENT_SESSION_ID", "reviewer-session")
            .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
        let out = surrender(&stub, ASSIGNED_HEAD, &tip, false);

        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout)["data"].clone();
        assert_eq!(data["status"], "surrendered");
        assert_eq!(data["head_sha"], ASSIGNED_HEAD);
        assert_eq!(data["handoff"]["assigned_head"], ASSIGNED_HEAD);
        assert_ne!(data["state_tip_digest"], tip);
    }
}

fn surrender(
    stub: &StubEnv,
    expected_head: &str,
    tip: &str,
    dry_run: bool,
) -> super::support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry_run {
        args.push("--dry-run");
    }
    args.extend([
        "pr",
        "review-handoff",
        "surrender",
        "7",
        "--expected-head",
        expected_head,
        "--expected-state",
        tip,
    ]);
    run_forge_cli(stub, &args)
}

fn set_env(stub: &mut StubEnv, key: &str, value: &str) {
    let mut found = false;
    for (name, current) in &mut stub.envs {
        if name == key {
            *current = value.to_owned();
            found = true;
        }
    }
    if !found {
        stub.envs.push((key.to_owned(), value.to_owned()));
    }
}

fn append_handoff(seeded: &mut Vec<ReviewStateRecord>, handoff: ReviewHandoff) {
    let head = handoff.assigned_head.clone();
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            &head,
            seeded.len() as u64,
            Some(seeded.last().unwrap().record_digest.clone()),
            ReviewStatePayload::ReviewHandoff { handoff },
        )
        .unwrap(),
    );
}

#[test]
fn moved_head_surrender_pins_expected_head_and_writer_guards() {
    let seeded = records(handoff(None), None);
    let tip = seeded.last().unwrap().record_digest.clone();

    let arbitrary_head = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
    let out = surrender(
        &arbitrary_head,
        "dddddddddddddddddddddddddddddddddddddddd",
        &tip,
        true,
    );
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "review_state_conflict"
    );

    for (actor, generation, expected_error) in [
        (Some("worker-session"), Some("1"), "review_writer_conflict"),
        (
            Some("reviewer-session"),
            Some("0"),
            "review_assignment_generation_conflict",
        ),
        (
            Some("reviewer-session"),
            Some("not-a-generation"),
            "review_assignment_generation_conflict",
        ),
        (
            Some("reviewer-session"),
            None,
            "review_assignment_generation_conflict",
        ),
    ] {
        let mut stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded);
        if let Some(actor) = actor {
            stub = stub.env("AGENT_SESSION_ID", actor);
        }
        if let Some(generation) = generation {
            stub = stub.env("AGENT_REVIEW_ASSIGNMENT_GENERATION", generation);
        } else {
            stub = stub.env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "");
        }
        let out = surrender(&stub, ASSIGNED_HEAD, &tip, true);
        assert_eq!(
            out.code, 65,
            "actor={actor:?} generation={generation:?}: {} {}",
            out.stdout, out.stderr
        );
        assert_eq!(parse_envelope(&out.stdout)["error"]["code"], expected_error);
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn moved_head_surrender_rejects_already_surrendered_assignment() {
    let mut assigned = handoff(None);
    assigned.surrendered = true;
    let mut seeded = records(handoff(None), None);
    append_handoff(&mut seeded, assigned);
    let tip = seeded.last().unwrap().record_digest.clone();
    let stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");

    let out = surrender(&stub, ASSIGNED_HEAD, &tip, true);
    assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "designated_reviewer_unavailable"
    );
}

#[test]
fn previous_assignment_observation_does_not_block_unobserved_head_surrender() {
    let mut seeded = records(handoff(None), Some(ASSIGNED_HEAD));
    let mut surrendered = handoff(None);
    surrendered.surrendered = true;
    append_handoff(&mut seeded, surrendered);
    let mut next = handoff(None);
    next.assignment_generation = 2;
    append_handoff(&mut seeded, next);
    let tip = seeded.last().unwrap().record_digest.clone();
    let stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");

    let out = surrender(&stub, ASSIGNED_HEAD, &tip, false);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["status"], "surrendered");
    assert_eq!(data["handoff"]["assignment_generation"], 2);
    assert_eq!(data["handoff"]["assigned_head"], CURRENT_HEAD);
    assert_eq!(data["handoff"]["surrendered"], true);
}

#[test]
fn moved_head_surrender_can_reassign_and_observe_in_generation_two() {
    let seeded = records(handoff(None), None);
    let tip = seeded.last().unwrap().record_digest.clone();
    let mut stub = writable_ledger(fixture(CURRENT_HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "1");
    let findings = stub.tempdir.path().join("generation-two-findings.json");
    fs::write(&findings, "[]").unwrap();
    let surrendered = surrender(&stub, ASSIGNED_HEAD, &tip, false);
    assert_eq!(
        surrendered.code, 0,
        "{} {}",
        surrendered.stdout, surrendered.stderr
    );
    let surrendered_tip = parse_envelope(&surrendered.stdout)["data"]["state_tip_digest"]
        .as_str()
        .unwrap()
        .to_string();

    set_env(&mut stub, "AGENT_SESSION_ID", "worker-session");
    set_env(&mut stub, "AGENT_REVIEWER_SESSION", "");
    let assigned = run_forge_cli(
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
            "assign",
            "7",
            "--reviewer-session",
            REVIEWER_UUID,
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            BASE_SHA,
            "--expected-head",
            CURRENT_HEAD,
            "--expected-state",
            &surrendered_tip,
        ],
    );
    assert_eq!(assigned.code, 0, "{} {}", assigned.stdout, assigned.stderr);
    assert_eq!(
        parse_envelope(&assigned.stdout)["data"]["handoff"]["assignment_generation"],
        2
    );

    set_env(&mut stub, "AGENT_SESSION_ID", REVIEWER_UUID);
    set_env(&mut stub, "AGENT_REVIEWER_SESSION", REVIEWER_UUID);
    set_env(&mut stub, "AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let observed = run_forge_cli(
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
            CURRENT_HEAD,
            "--auto-state",
            "--findings-file",
            findings.to_str().unwrap(),
        ],
    );
    assert_eq!(observed.code, 0, "{} {}", observed.stdout, observed.stderr);
    assert_eq!(parse_envelope(&observed.stdout)["data"]["appended"], true);
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
