//! Retired-coordinator takeover integration coverage.
use std::process::Command;

use super::*;
use pretty_assertions::assert_eq;

fn coordinator_board(stub: StubEnv, state: &str, reason: &str) -> StubEnv {
    let now = jiff::Timestamp::now().to_string();
    let last_activity = (jiff::Timestamp::now() - jiff::SignedDuration::from_hours(3)).to_string();
    let board = json!({"ok":true,"data":{"mode":"local","board":{
        "schema_version":"agent-session.board-view.v1", "generated_at":now,
        "truncated":false, "machines":[{"machine":"source", "available":true,"last_seen_at":now}],
        "records":[{"session_id":COORDINATOR_UUID, "machine":"source", "state":state,
            "last_terminal_activity_at":last_activity,
            "runtime_status":if state == "live" { json!("running") } else if state == "stopped" { json!("stopped") } else { Value::Null },
            "closed_at":if state == "closed" { json!(now) } else { Value::Null },
            "close_reason":reason}]
    }}});
    let path = stub.write_stub(
        "agent-session",
        &format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
    );
    stub.env("FORGE_CLI_AGENT_SESSION_BIN", path.to_string_lossy())
        .env("AGENT_SESSION_ID", "successor-session")
}

fn takeover(
    stub: &StubEnv,
    head: &str,
    base: &str,
    tip: &str,
    selector: &str,
    dry: bool,
) -> super::super::support::CmdOutput {
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
        "review-handoff",
        "recover",
        "7",
        "--expected-head",
        head,
        "--expected-state",
        tip,
        "--base-sha",
        base,
        "--reason",
        "coordinator-retired",
        "--coordinator-session",
        selector,
    ]);
    run_with_supervisor_config(stub, &args, true)
}

fn run_with_supervisor_config(
    stub: &StubEnv,
    args: &[&str],
    allowlisted: bool,
) -> super::super::support::CmdOutput {
    run_with_supervisor_config_on_remote(stub, args, allowlisted, "origin")
}

fn run_with_supervisor_config_on_remote(
    stub: &StubEnv,
    args: &[&str],
    allowlisted: bool,
    remote: &str,
) -> super::super::support::CmdOutput {
    let stale_after = if remote == "invalid" { "31d" } else { "2h" };
    run_with_supervisor_policy(stub, args, allowlisted, remote, stale_after)
}

fn run_with_supervisor_policy(
    stub: &StubEnv,
    args: &[&str],
    allowlisted: bool,
    remote: &str,
    stale_after: &str,
) -> super::super::support::CmdOutput {
    let repo = stub.tempdir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let actor = stub
        .envs
        .iter()
        .rev()
        .find(|(key, _)| key == "AGENT_SESSION_ID")
        .map(|(_, value)| value.as_str())
        .unwrap_or("successor-session");
    let supervisors = if allowlisted {
        format!("[\"{}\"]", review_state::sha256_digest(actor.as_bytes()))
    } else {
        "[]".to_string()
    };
    fs::write(
        repo.join(".forge-cli.toml"),
        format!("[review_handoff]\nsupervisors = {supervisors}\nstale_after = \"{stale_after}\"\n"),
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["add", ".forge-cli.toml"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture config"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    let mut base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert!(
        Command::new("git")
            .args([
                "remote",
                "add",
                remote,
                if remote == "wrong" {
                    "https://github.com/acme/other.git"
                } else if remote == "host" {
                    "https://gitlab.com/acme/widgets.git"
                } else if remote == "mixed" {
                    "https://github.com/Acme/Widgets.git"
                } else {
                    "https://github.com/acme/widgets.git"
                },
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    if remote == "stale" || remote == "behind" {
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "advance remote ref"
                ])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    // `stale` caches a branch tip newer than the provider base; `behind` makes the
    // provider base newer than the cached tip; `uncached` never fetched the base.
    let cached = if remote == "behind" {
        base.clone()
    } else {
        "HEAD".to_string()
    };
    if remote == "behind" {
        base = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
    }
    if remote != "uncached" {
        assert!(
            Command::new("git")
                .args([
                    "update-ref",
                    &format!("refs/remotes/{remote}/main"),
                    &cached
                ])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            if *arg == OLD {
                base.clone()
            } else {
                (*arg).to_string()
            }
        })
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    super::super::support::run_forge_cli_in_with_env(
        stub,
        &args,
        Some(&repo),
        &[("PROVIDER_BASE", &base), ("PROVIDER_BASE_REF", "main")],
    )
}

fn supervisor_takeover(
    stub: &StubEnv,
    head: &str,
    base: &str,
    tip: &str,
    allowlisted: bool,
    dry: bool,
) -> super::super::support::CmdOutput {
    supervisor_takeover_on_remote(stub, head, base, tip, allowlisted, dry, "origin")
}

fn supervisor_takeover_on_remote(
    stub: &StubEnv,
    head: &str,
    base: &str,
    tip: &str,
    allowlisted: bool,
    dry: bool,
    remote: &str,
) -> super::super::support::CmdOutput {
    let mut args = vec![
        "--remote",
        remote,
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
        "review-handoff",
        "recover",
        "7",
        "--expected-head",
        head,
        "--expected-state",
        tip,
        "--base-sha",
        base,
        "--reason",
        "coordinator-retired",
    ]);
    run_with_supervisor_config_on_remote(stub, &args, allowlisted, remote)
}

#[test]
fn supervisor_takeover_requires_configured_identity_and_fresh_stale_retirement_proof() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let supervisor = "successor-session";
    let supervisor_digest = review_state::sha256_digest(supervisor.as_bytes());
    for (state, activity_age_hours, allowlisted, expected_tip, expected) in [
        ("live", 3, true, "actual", "review_coordinator_live"),
        ("stopped", 1, true, "actual", "review_coordinator_unproven"),
        ("stopped", 3, false, "actual", "review_writer_conflict"),
        ("stopped", 3, true, "actual", "accepted"),
        ("stopped", 3, true, "none", "review_state_conflict"),
    ] {
        let mut stub = coordinator_board(fixture(HEAD, &seeded, vec![]), state, "");
        let now = jiff::Timestamp::now();
        let activity = (now - jiff::SignedDuration::from_hours(activity_age_hours)).to_string();
        let state_name = state;
        let board = json!({"ok":true,"data":{"mode":"local","board":{
            "schema_version":"agent-session.board-view.v1", "generated_at":now.to_string(),
            "truncated":false,"machines":[{"machine":"source","available":true,"last_seen_at":now.to_string()}],
            "records":[{"session_id":COORDINATOR_UUID,"session_incarnation":"retired-incarnation",
                "machine":"source","state":state_name,
                "runtime_status":if state_name == "live" { json!("running") } else { json!("stopped") },
                "last_terminal_activity_at":activity,
                "closed_at":Value::Null,"close_reason":Value::Null}]
        }}});
        let board_stub = stub.write_stub(
            "agent-session",
            &format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
        );
        stub.envs.push((
            "FORGE_CLI_AGENT_SESSION_BIN".into(),
            board_stub.to_string_lossy().into_owned(),
        ));
        stub.envs
            .push(("AGENT_SESSION_ID".into(), "successor-session".into()));
        let expected_tip = if expected_tip == "actual" {
            tip.as_str()
        } else {
            expected_tip
        };
        let out = supervisor_takeover(&stub, HEAD, OLD, expected_tip, allowlisted, true);
        if !allowlisted {
            assert!(out.stdout.contains("reviewer-declined"), "{}", out.stdout);
            assert!(
                out.stdout.contains("[review_handoff].supervisors"),
                "{}",
                out.stdout
            );
        }
        if expected == "accepted" {
            assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
            let handoff = parse_envelope(&out.stdout)["data"]["handoff"].clone();
            assert_eq!(handoff["assignment_generation"], 2);
            assert_eq!(handoff["coordinator_digest"], supervisor_digest);
        } else {
            assert!(
                out.code == 65 && out.stdout.contains(expected),
                "state={state} age={activity_age_hours} allowlisted={allowlisted}; expected={expected}; got={} stderr={}",
                out.stdout,
                out.stderr
            );
        }
    }
}

#[test]
fn supervisor_takeover_rechecks_policy_from_the_selected_remote() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "upstream");
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
}

#[test]
fn supervisor_takeover_append_accepts_case_insensitive_github_repository_slug() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = writable_ledger(
        coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", ""),
        &seeded,
    );
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, false, "mixed");
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let saved: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), 2);
    assert_eq!(chain.records.last().unwrap().assignment_generation, Some(2));
}

#[test]
fn supervisor_takeover_rejects_a_remote_for_another_repository() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "wrong");
    assert!(out.code == 65 && out.stdout.contains("review_coordinator_unproven"));
}

#[test]
fn supervisor_takeover_rejects_a_remote_on_another_provider_host() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "host");
    assert!(out.code == 65 && out.stdout.contains("review_coordinator_unproven"));
}

#[test]
fn supervisor_takeover_rejects_a_remote_ref_that_moved_from_provider_base() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "stale");
    assert!(out.code == 65 && out.stdout.contains("review_coordinator_unproven"));
}

#[test]
fn supervisor_takeover_rejects_invalid_explicit_staleness_policy() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded[0].record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "");
    let out = supervisor_takeover_on_remote(&stub, HEAD, OLD, tip, true, true, "invalid");
    assert!(out.code == 65 && out.stdout.contains("review_coordinator_unproven"));
}

#[test]
fn retired_coordinator_takeover_preview_transfers_only_coordinator_authority() {
    let seeded = records(retired_coordinator_handoff(), Some(HEAD));
    let tip = &seeded.last().unwrap().record_digest;
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted")
        .env("PROVIDER_BASE", HEAD);
    let out = takeover(
        &stub,
        HEAD,
        OLD,
        tip,
        &format!("{COORDINATOR_UUID}@source"),
        true,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["state_tip_digest"], *tip);
    assert_eq!(
        data["handoff"]["coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    assert_eq!(
        data["handoff"]["reviewer_digest"],
        review_state::sha256_digest(b"reviewer-session")
    );
    assert_eq!(data["handoff"]["assignment_generation"], 2);
    let policy_base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "refs/remotes/origin/main"])
            .current_dir(stub.tempdir.path().join("repo"))
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(data["handoff"]["base_sha"], policy_base);
    assert_eq!(data["handoff"]["returned_reason"], "coordinator-retired");
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["old_coordinator_digest"],
        review_state::sha256_digest(COORDINATOR_UUID.as_bytes())
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["new_coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["reason"],
        "coordinator-retired"
    );
    assert_eq!(
        data["handoff"]["coordinator_transfer"]["lifecycle_source_digest"],
        review_state::sha256_digest(b"source")
    );
    assert!(!out.stdout.contains("\"source\""));
    assert!(!out.stdout.contains(COORDINATOR_UUID));
    assert!(!out.stdout.contains("successor-session"));
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn retired_coordinator_takeover_refuses_live_unproven_and_wrong_identity() {
    let seeded = records(retired_coordinator_handoff(), None);
    let tip = &seeded.last().unwrap().record_digest;
    for (state, reason, selector, kind) in [
        ("live", "", COORDINATOR_UUID, "review_coordinator_live"),
        (
            "closed",
            "vanished",
            COORDINATOR_UUID,
            "review_coordinator_unproven",
        ),
        (
            "closed",
            "deleted",
            OTHER_COORDINATOR_UUID,
            "review_assignment_conflict",
        ),
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), state, reason);
        let out = takeover(&stub, HEAD, OLD, tip, selector, true);
        assert_refusal(&out, 65, kind);
    }
}

#[test]
fn retired_coordinator_takeover_requires_exact_head_base_and_tip() {
    let seeded = records(handoff(None), None);
    let tip = &seeded.last().unwrap().record_digest;
    for (head, base, expected_tip, kind) in [
        (OLD, OLD, tip.as_str(), "review_state_conflict"),
        (HEAD, HEAD, tip.as_str(), "review_scope_changed"),
        (HEAD, OLD, "none", "review_state_conflict"),
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "archived");
        let out = takeover(&stub, head, base, expected_tip, "worker-session", true);
        assert_refusal(&out, 65, kind);
    }
}

#[test]
fn retired_coordinator_takeover_persists_findings_and_allows_reassignment() {
    let mut seeded = records(retired_coordinator_handoff(), None);
    let findings: Vec<review_state::ReviewFindingObservation> = serde_json::from_value(json!([
        {"fingerprint":"testing:handoff:open-finding", "blocking":true, "threads":["thread-1"]}
    ]))
    .unwrap();
    let original = review_state::observe_review_loop(None, HEAD, &findings)
        .unwrap()
        .state;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(seeded[0].record_digest.clone()),
            ReviewStatePayload::ReviewLoop {
                state: original.clone(),
            },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let mut surrendered = retired_coordinator_handoff();
    surrendered.surrendered = true;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            2,
            Some(seeded[1].record_digest.clone()),
            ReviewStatePayload::ReviewHandoff {
                handoff: surrendered,
            },
        )
        .unwrap(),
    );
    let stub = writable_ledger(
        coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "archived"),
        &seeded,
    )
    .env("PROVIDER_BASE", HEAD)
    .env("AGENT_REVIEWER_SESSION", "");
    let out = takeover(
        &stub,
        HEAD,
        OLD,
        &seeded.last().unwrap().record_digest,
        &format!("{COORDINATOR_UUID}@source"),
        false,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let tip = parse_envelope(&out.stdout)["data"]["state_tip_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let saved: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), seeded.len() + 1);
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&original)
    );
    assert!(
        saved
            .last()
            .unwrap()
            .contains("Coordinator ownership transferred")
    );
    // Coordinator transfer alone never authorizes reviewer appends or publication.
    let blocked = observe(&stub, HEAD);
    assert_eq!(
        parse_envelope(&blocked.stdout)["data"]["preflight_ok"],
        false
    );
    assert!(blocked.stdout.contains("designated_reviewer_unavailable"));
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
            "assign",
            "7",
            "--reviewer-session",
            REVIEWER_UUID,
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            HEAD,
            "--expected-head",
            HEAD,
            "--expected-state",
            &tip,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["handoff"]["assignment_generation"], 3);
    assert!(data["handoff"].get("coordinator_transfer").is_none());
    let blocked = observe(&stub, HEAD);
    assert_eq!(
        parse_envelope(&blocked.stdout)["data"]["preflight_ok"],
        false
    );
    assert!(blocked.stdout.contains("review_writer_conflict"));
    let published = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review",
            "7",
            "--decision",
            "comments-only",
            "--submit-review",
            "--expected-head",
            HEAD,
            "--comment",
            "review evidence",
            "--lens",
            "testing",
        ],
    );
    assert_refusal(&published, 65, "review_writer_conflict");
    let reviewer = stub
        .env("AGENT_SESSION_ID", REVIEWER_UUID)
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "3");
    let findings_file = reviewer.tempdir.path().join("open-findings.json");
    fs::write(&findings_file, r#"[{"lifecycle_fingerprint":"testing:handoff:open-finding","disposition":"open","blocking":true,"threads":["thread-1"]}]"#).unwrap();
    let out = run_forge_cli(
        &reviewer,
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
            HEAD,
            "--auto-state",
            "--findings-file",
            findings_file.to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["appended"], true, "{}", out.stdout);
    let saved: Vec<String> = serde_json::from_str(
        &fs::read_to_string(reviewer.tempdir.path().join("ledger.json")).unwrap(),
    )
    .unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(&original)
    );
}

#[test]
fn retired_coordinator_takeover_rechecks_lifecycle_before_posting() {
    let seeded = records(retired_coordinator_handoff(), None);
    let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
    let inner = stub.tempdir.path().join("board-inner");
    fs::rename(stub.tempdir.path().join("agent-session"), &inner).unwrap();
    let counter = stub.tempdir.path().join("board-read");
    let script = format!(
        r#"#!/bin/sh
if [ -e '{counter}' ]; then
  '{inner}' "$@" | sed 's/"closed"/"live"/g'
else
  touch '{counter}'
  exec '{inner}' "$@"
fi
"#,
        counter = counter.display(),
        inner = inner.display()
    );
    stub.write_stub("agent-session", &script);
    let out = takeover(
        &stub,
        HEAD,
        OLD,
        &seeded[0].record_digest,
        COORDINATOR_UUID,
        false,
    );
    assert_refusal(&out, 65, "review_coordinator_live");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

#[test]
fn retired_coordinator_takeover_refuses_incomplete_or_stale_board_evidence() {
    let seeded = records(retired_coordinator_handoff(), None);
    for change in [
        "absent",
        "offline",
        "truncated",
        "stale",
        "malformed",
        "failed",
        "duplicate-live",
    ] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let now = jiff::Timestamp::now().to_string();
        let mut board = json!({"ok":true,"data":{"board":{
            "schema_version":"agent-session.board-view.v1", "generated_at":now, "truncated":false,
            "machines":[{"machine":"source", "available":true, "last_seen_at":now}],
            "records":[{"session_id":COORDINATOR_UUID, "machine":"source", "state":"closed",
                "runtime_status":null,"closed_at":now,"close_reason":"deleted"}]
        }}});
        let b = &mut board["data"]["board"];
        match change {
            "absent" => b["records"] = json!([]),
            "offline" => b["machines"][0]["available"] = json!(false),
            "truncated" => b["truncated"] = json!(true),
            "stale" => b["machines"][0]["last_seen_at"] = json!("2020-01-01T00:00:00Z"),
            "duplicate-live" => b["records"]
                .as_array_mut()
                .unwrap()
                .push(json!({"session_id":COORDINATOR_UUID,"state":"live"})),
            _ => (),
        }
        let script = match change {
            "malformed" => "#!/bin/sh\necho private-evidence-canary\n".into(),
            "failed" => "#!/bin/sh\necho private-evidence-canary >&2\nexit 1\n".into(),
            _ => format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
        };
        stub.write_stub("agent-session", &script);
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            COORDINATOR_UUID,
            true,
        );
        let kind = if change == "duplicate-live" {
            "review_coordinator_live"
        } else {
            "review_coordinator_unproven"
        };
        assert_refusal(&out, 65, kind);
        let error = parse_envelope(&out.stdout)["error"].clone();
        assert!(error["details"]["retryable"].is_boolean());
        assert!(error["details"]["next_action"].is_string());
        assert!(error["details"]["recovery"].is_object());
        assert!(!out.stdout.contains("private-evidence-canary"));
        assert!(!out.stderr.contains("private-evidence-canary"));
    }
}

#[test]
fn retired_coordinator_takeover_chain_validates_transfer_and_racing_writers() {
    let root = records(handoff(None), None).remove(0);
    let mut value = serde_json::to_value(handoff(None)).unwrap();
    value["coordinator_digest"] = json!(review_state::sha256_digest(b"successor-session"));
    value["returned_reason"] = json!("coordinator-retired");
    value["assignment_generation"] = json!(2);
    value["coordinator_transfer"] = json!({
        "old_coordinator_digest":review_state::sha256_digest(b"worker-session"),
        "new_coordinator_digest":review_state::sha256_digest(b"successor-session"),
        "lifecycle_source_digest":review_state::sha256_digest(b"source"),
        "reason":"coordinator-retired"
    });
    let transfer = |v: Value| {
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(root.record_digest.clone()),
            ReviewStatePayload::ReviewHandoff {
                handoff: serde_json::from_value(v).unwrap(),
            },
        )
        .unwrap()
    };
    let recovery = transfer(value.clone());
    let observation = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        1,
        Some(root.record_digest.clone()),
        ReviewStatePayload::ReviewLoop {
            state: review_state::observe_review_loop(None, HEAD, &[])
                .unwrap()
                .state,
        },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    let markers = [
        root.marker().unwrap(),
        observation.marker().unwrap(),
        recovery.marker().unwrap(),
    ];
    let chain =
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.tip_digest, Some(recovery.record_digest.clone()));
    assert_eq!(chain.ignored_stale_records, vec![observation.record_digest]);
    let mut invalid = value.clone();
    invalid["coordinator_transfer"]["old_coordinator_digest"] =
        json!(review_state::sha256_digest(b"unrelated-session"));
    let markers = [root.marker().unwrap(), transfer(invalid).marker().unwrap()];
    assert!(
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).is_err()
    );
    let mut competing = value;
    competing["coordinator_digest"] = json!(review_state::sha256_digest(b"another-successor"));
    competing["coordinator_transfer"]["new_coordinator_digest"] =
        competing["coordinator_digest"].clone();
    let markers = [
        root.marker().unwrap(),
        recovery.marker().unwrap(),
        transfer(competing).marker().unwrap(),
    ];
    assert!(
        review_state::parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).is_err()
    );
    assert!(
        !serde_json::to_string(&handoff(None))
            .unwrap()
            .contains("coordinator_transfer")
    );
}

#[test]
fn retired_coordinator_takeover_rechecks_provider_scope_before_posting() {
    let seeded = records(retired_coordinator_handoff(), None);
    for changed in ["head", "base", "tip"] {
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let inner = stub.tempdir.path().join("gh-inner");
        fs::rename(stub.tempdir.path().join("gh"), &inner).unwrap();
        let counter = stub.tempdir.path().join("scope-read");
        let script = format!(
            r#"#!/bin/sh
case "$1 $2" in
  "pr view")
    if [ '{changed}' = head ]; then
      if [ -e '{counter}' ]; then '{inner}' "$@" | sed 's/{HEAD}/{OLD}/g'; exit 0; fi
      touch '{counter}'
    fi ;;
  "api repos/acme/widgets/pulls/7")
    if [ '{changed}' = base ]; then
      if [ -e '{counter}' ]; then echo '{HEAD}'; exit 0; fi
      touch '{counter}'
    fi ;;
  "api graphql")
    if [ '{changed}' = tip ]; then
      if [ -e '{counter}' ]; then
        echo '{{"data":{{"viewer":{{"login":"review-app[bot]"}},"repository":{{"pullRequest":{{"comments":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}}}'
        exit 0
      fi
      touch '{counter}'
    fi ;;
esac
exec '{inner}' "$@"
"#,
            changed = changed,
            counter = counter.display(),
            inner = inner.display()
        );
        let stub = stub.gh_stub(&script);
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            COORDINATOR_UUID,
            false,
        );
        assert_refusal(
            &out,
            65,
            if changed == "base" {
                "review_scope_changed"
            } else {
                "review_state_conflict"
            },
        );
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn retired_coordinator_takeover_binds_the_selected_lifecycle_source() {
    let seeded = records(retired_coordinator_handoff(), None);
    for scenario in ["unavailable", "live-namesake", "closed-namesake"] {
        let target_closed = scenario != "unavailable";
        let stub = coordinator_board(fixture(HEAD, &seeded, vec![]), "closed", "deleted");
        let now = jiff::Timestamp::now().to_string();
        let mut board = json!({"ok":true,"data":{"board":{
            "schema_version":"agent-session.board-view.v1", "generated_at":now, "truncated":false,
            "machines":[{"machine":"target", "available":target_closed, "last_seen_at":now},
                {"machine":"namesake", "available":true, "last_seen_at":now}],
            "records":if target_closed { json!([
                {"session_id":COORDINATOR_UUID, "machine":"target", "state":"closed", "runtime_status":null,
                    "closed_at":now,"close_reason":"deleted"},
                {"session_id":COORDINATOR_UUID, "machine":"namesake", "state":"live", "runtime_status":"running"}
            ]) } else { json!([
                {"session_id":COORDINATOR_UUID, "machine":"namesake", "state":"closed", "runtime_status":null,
                    "closed_at":now,"close_reason":"deleted"}
            ]) }
        }}});
        if scenario == "closed-namesake" {
            board["data"]["board"]["records"][1] = json!({
                "session_id":COORDINATOR_UUID, "machine":"namesake", "state":"closed", "runtime_status":null,
                "closed_at":now,"close_reason":"deleted"
            });
        }
        stub.write_stub(
            "agent-session",
            &format!("#!/bin/sh\ncat <<'JSON'\n{board}\nJSON\n"),
        );
        let out = takeover(
            &stub,
            HEAD,
            OLD,
            &seeded[0].record_digest,
            &format!("{COORDINATOR_UUID}@target"),
            true,
        );
        if scenario == "live-namesake" {
            assert_refusal(&out, 65, "review_coordinator_live");
        } else {
            assert_refusal(&out, 65, "review_coordinator_unproven");
        }
    }
}

#[path = "retired_coordinator/diagnostics.rs"]
mod diagnostics;

#[path = "retired_coordinator/deleted_producer.rs"]
mod deleted_producer;
