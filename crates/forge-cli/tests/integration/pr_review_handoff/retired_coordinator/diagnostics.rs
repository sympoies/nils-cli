//! Each retired-coordinator refusal names its own missing prerequisite and remedy.
use super::*;
use pretty_assertions::assert_eq;

fn refusal(stub: &StubEnv, remote: &str, allowlisted: bool, tip: &str) -> Value {
    let out = supervisor_takeover_on_remote(stub, HEAD, OLD, tip, allowlisted, true, remote);
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    parse_envelope(&out.stdout)["error"].clone()
}

/// A board the supervisor can recover from; each case breaks one prerequisite.
fn board(mutate: impl FnOnce(&mut Value)) -> Value {
    let now = jiff::Timestamp::now();
    let activity = (now - jiff::SignedDuration::from_hours(3)).to_string();
    let mut value = json!({"ok":true,"data":{"mode":"local","board":{
        "schema_version":"agent-session.board-view.v1","generated_at":now.to_string(),
        "truncated":false,
        "machines":[{"machine":"source","available":true,"last_seen_at":now.to_string()}],
        "records":[{"session_id":COORDINATOR_UUID,"machine":"source","state":"stopped",
            "runtime_status":"stopped","last_terminal_activity_at":activity,
            "closed_at":Value::Null,"close_reason":Value::Null}]
    }}});
    mutate(&mut value["data"]["board"]);
    value
}

#[test]
fn policy_refusals_name_the_missing_base_or_config_prerequisite() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    for (remote, prerequisite, retryable, next_action, hint) in [
        (
            "wrong",
            "remote_repository_mismatch",
            false,
            "select_matching_remote",
            "checkout of this repository",
        ),
        (
            "host",
            "remote_repository_mismatch",
            false,
            "select_matching_remote",
            "checkout of this repository",
        ),
        (
            "uncached",
            "base_ref_missing",
            true,
            "fetch_base_ref",
            "git fetch uncached main",
        ),
        (
            "behind",
            "base_ref_stale",
            true,
            "fetch_base_ref",
            "git fetch behind main",
        ),
        (
            "stale",
            "provider_base_behind_cache",
            true,
            "update_pr_branch",
            "update the pull request branch",
        ),
        (
            "invalid",
            "policy_config_invalid",
            false,
            "fix_handoff_policy_config",
            "stale_after",
        ),
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
        let error = refusal(&stub, remote, true, tip);
        let context = format!("{remote}: {error}");
        assert_eq!(error["code"], "review_coordinator_unproven", "{context}");
        let details = &error["details"];
        assert_eq!(details["prerequisite"], prerequisite, "{context}");
        assert_eq!(details["retryable"], retryable, "{context}");
        assert_eq!(details["next_action"], next_action, "{context}");
        assert!(details["recovery"].is_object(), "{context}");
        assert!(
            error["message"].as_str().unwrap().contains(hint),
            "{context}"
        );
    }
}

#[test]
fn an_unconfigured_supervisor_list_is_named_distinctly_from_a_missing_member() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let error = refusal(&stub, "origin", false, tip);
    assert_eq!(error["code"], "review_writer_conflict");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("no repository-configured handoff supervisor"),
        "{message}"
    );
    assert!(
        message.contains("[review_handoff].supervisors"),
        "{message}"
    );
}

#[test]
fn lifecycle_refusals_name_the_missing_evidence_and_who_can_supply_it() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    type Case = (&'static str, fn(&mut Value), &'static str, bool);
    let cases: [Case; 9] = [
        (
            "board unreadable",
            |b| b["schema_version"] = json!("other"),
            "session_board_unavailable",
            true,
        ),
        (
            "board truncated",
            |b| b["truncated"] = json!(true),
            "session_board_incomplete",
            true,
        ),
        (
            "board stale",
            |b| b["generated_at"] = json!("2020-01-01T00:00:00Z"),
            "session_board_incomplete",
            true,
        ),
        (
            "coordinator absent",
            |b| b["records"] = json!([]),
            "coordinator_not_on_board",
            false,
        ),
        (
            "source offline",
            |b| b["machines"][0]["available"] = json!(false),
            "lifecycle_source_unavailable",
            true,
        ),
        (
            "not terminal",
            |b| {
                b["records"][0]["state"] = json!("closed");
                b["records"][0]["runtime_status"] = Value::Null;
                b["records"][0]["close_reason"] = json!("vanished");
            },
            "coordinator_not_terminal",
            false,
        ),
        (
            "terminal activity missing",
            |b| {
                b["records"][0]["state"] = json!("closed");
                b["records"][0]["runtime_status"] = Value::Null;
                b["records"][0]["close_reason"] = json!("deleted");
                b["records"][0]["last_terminal_activity_at"] = Value::Null;
            },
            "terminal_activity_missing",
            false,
        ),
        (
            "activity not stale",
            |b| {
                b["records"][0]["last_terminal_activity_at"] = json!(
                    (jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1)).to_string()
                );
            },
            "coordinator_not_stale",
            true,
        ),
        (
            "ambiguous source",
            |b| {
                let mut other = b["records"][0].clone();
                other["machine"] = json!("elsewhere");
                b["records"].as_array_mut().unwrap().push(other);
            },
            "coordinator_source_ambiguous",
            false,
        ),
    ];
    // Control: the unmodified board recovers, so each refusal below is its mutation.
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    stub.write_stub(
        "agent-session",
        &format!("#!/bin/sh\ncat <<'JSON'\n{}\nJSON\n", board(|_| ())),
    );
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "origin");
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    for (name, mutate, prerequisite, retryable) in cases {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
        stub.write_stub(
            "agent-session",
            &format!("#!/bin/sh\ncat <<'JSON'\n{}\nJSON\n", board(mutate)),
        );
        let error = refusal(&stub, "origin", true, tip);
        let context = format!("{name}: {error}");
        assert_eq!(error["code"], "review_coordinator_unproven", "{context}");
        assert_eq!(error["details"]["prerequisite"], prerequisite, "{context}");
        assert_eq!(error["details"]["retryable"], retryable, "{context}");
        assert!(error["details"]["next_action"].is_string(), "{context}");
        assert!(error["details"]["recovery"].is_object(), "{context}");
    }
}
