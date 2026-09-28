//! Operator cleanup of orphaned Main Agent runs: `main-agent runs orphaned`
//! lists runs whose controller and workers are all gone, and
//! `main-agent runs close-orphaned` terminalizes exactly the reviewed plan.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use nils_test_support::bin;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const OLD: &str = "2026-07-01T00:00:00Z";
/// A far-future activity time keeps a run inside any `--older-than` window
/// without depending on the wall clock.
const RECENT: &str = "2999-01-01T00:00:00Z";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Output {
    code: i32,
    json: Value,
}

fn main_agent(state_dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(bin::resolve("main-agent"))
        .arg("--state-dir")
        .arg(state_dir)
        .args(args)
        .env_remove("AGENT_SESSION_CAPABILITY_FILE")
        .env_remove("AGENT_SESSION_CHECKPOINT_FILE")
        .output()
        .expect("run main-agent");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let json = serde_json::from_str(&stdout).unwrap_or_else(|_| {
        panic!(
            "args={args:?} stdout={stdout} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    Output {
        code: output.status.code().expect("exit code"),
        json,
    }
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).expect("private directory");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("directory mode");
}

fn private_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("file mode");
}

fn session_ref(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "session_incarnation": format!("{session_id}-incarnation"),
        "session_created_at": OLD
    })
}

fn run(run_id: &str, controller: &str, updated_at: &str) -> Value {
    json!({
        "schema_version": "agent-session.orchestration-run.v1",
        "run_id": run_id,
        "revision": 3,
        "state": "active",
        "tier": "direct",
        "objective_summary": format!("Fixture objective for {run_id}"),
        "objective_packet_digest": DIGEST,
        "controller": session_ref(controller),
        "durable_refs": [],
        "ephemeral": false,
        "created_at": OLD,
        "updated_at": updated_at
    })
}

fn assignment(
    assignment_id: &str,
    run_id: &str,
    controller: &str,
    state: &str,
    worker: Option<&str>,
) -> Value {
    let mut value = json!({
        "schema_version": "agent-session.orchestration-assignment.v3",
        "assignment_id": assignment_id,
        "run_id": run_id,
        "revision": 5,
        "state": state,
        "task_summary": format!("Fixture lane {assignment_id}"),
        "private_packet_digest": DIGEST,
        "primary_manager": session_ref(controller),
        "collaborators": [],
        "borrowed_by": [],
        "scopes": [],
        "durable_refs": [],
        "created_at": OLD,
        "updated_at": OLD
    });
    if let Some(worker) = worker {
        value["worker"] = session_ref(worker);
    }
    value
}

fn claim(session_id: &str) -> Value {
    json!({
        "schema_version": "agent-session.work-context.v1",
        "session_id": session_id,
        "session_incarnation": format!("{session_id}-incarnation"),
        "claim_id": format!("{session_id}-claim"),
        "revision": 1,
        "state": "active",
        "intent": "implementation",
        "tier": "direct",
        "repositories": ["example/repository"],
        "worktrees": [],
        "provider_refs": [],
        "plan_refs": [],
        "scopes": [{
            "kind": "path-prefix",
            "repository": "example/repository",
            "value": format!("docs/{session_id}")
        }],
        "summary": "Orphaned run fixture claim",
        "updated_at": OLD,
        "expires_at": "2999-01-01T00:00:00Z",
        "expires_at_epoch": i64::MAX
    })
}

/// Seed one closable orphan beside every shape the cleanup must refuse or
/// ignore, so each test observes the whole decision surface at once.
fn seed(state_dir: &Path) {
    private_dir(state_dir);
    private_dir(&state_dir.join("orchestration"));
    private_dir(&state_dir.join("coordination"));
    private_dir(&state_dir.join("sessions"));
    for live in ["main-live", "worker-live-session", "worker-cleanup-pending"] {
        private_dir(&state_dir.join("sessions").join(live));
    }
    let mut runs = serde_json::Map::new();
    let mut assignments = serde_json::Map::new();
    let mut add_run = |value: Value| {
        runs.insert(value["run_id"].as_str().unwrap().to_string(), value);
    };
    add_run(run("run-orphan", "main-gone", OLD));
    add_run(run("run-controller-live", "main-live", OLD));
    add_run(run("run-controller-claim", "main-claimed", OLD));
    add_run(run("run-worker-session", "main-gone-2", OLD));
    add_run(run("run-worker-claim", "main-gone-3", OLD));
    add_run(run("run-launch-pending", "main-gone-4", OLD));
    add_run(run("run-operation", "main-gone-5", OLD));
    add_run(run("run-receipt", "main-gone-6", OLD));
    add_run(run("run-recent", "main-gone-7", RECENT));
    add_run(run("run-cleanup-pending", "main-gone-9", OLD));
    add_run(run("run-canary-fence", "main-gone-10", OLD));
    add_run(run("run-quarantined", "main-gone-11", OLD));
    add_run(run("run-worker-receipt", "main-gone-12", OLD));
    add_run(run("run-assignment-recent", "main-gone-13", OLD));
    add_run(run("run-bad-time", "main-gone-14", "not-a-timestamp"));
    let mut closed = run("run-closed", "main-gone-8", OLD);
    closed["state"] = json!("closed");
    add_run(closed);
    let mut add_assignment = |value: Value| {
        assignments.insert(value["assignment_id"].as_str().unwrap().to_string(), value);
    };
    add_assignment(assignment(
        "orphan-working",
        "run-orphan",
        "main-gone",
        "working",
        Some("worker-gone-a"),
    ));
    add_assignment(assignment(
        "orphan-accepted",
        "run-orphan",
        "main-gone",
        "accepted",
        Some("worker-gone-b"),
    ));
    add_assignment(assignment(
        "orphan-cancelled",
        "run-orphan",
        "main-gone",
        "cancelled",
        Some("worker-gone-c"),
    ));
    add_assignment(assignment(
        "orphan-never-launched",
        "run-orphan",
        "main-gone",
        "cancelled",
        None,
    ));
    add_assignment(assignment(
        "cleanup-pending-released",
        "run-cleanup-pending",
        "main-gone-9",
        "released",
        Some("worker-cleanup-pending"),
    ));
    add_assignment(assignment(
        "controller-live-working",
        "run-controller-live",
        "main-live",
        "working",
        Some("worker-gone-d"),
    ));
    add_assignment(assignment(
        "worker-session-blocked",
        "run-worker-session",
        "main-gone-2",
        "blocked",
        Some("worker-live-session"),
    ));
    add_assignment(assignment(
        "worker-claim-working",
        "run-worker-claim",
        "main-gone-3",
        "working",
        Some("worker-claimed"),
    ));
    add_assignment(assignment(
        "launch-pending-starting",
        "run-launch-pending",
        "main-gone-4",
        "starting",
        None,
    ));
    let mut fenced = assignment(
        "operation-working",
        "run-operation",
        "main-gone-5",
        "working",
        Some("worker-gone-e"),
    );
    fenced["submit_recovery"] = json!({
        "schema_version": "main-agent.submit-recovery.v1",
        "attempt_id": "attempt-one",
        "origin": "explicit",
        "session_incarnation": "worker-gone-e-incarnation",
        "reserved_revision": 5,
        "state": "attempting",
        "attempt_count": 1,
        "result": "Submit recovery reserved",
        "attempted_at": OLD,
        "updated_at": OLD
    });
    add_assignment(fenced);
    add_assignment(assignment(
        "canary-working",
        "run-canary-fence",
        "main-gone-10",
        "working",
        Some("worker-gone-h"),
    ));
    let mut quarantined = assignment(
        "quarantined-working",
        "run-quarantined",
        "main-gone-11",
        "working",
        Some("worker-gone-i"),
    );
    quarantined["submit_recovery"] = json!({
        "schema_version": "main-agent.submit-recovery.v1",
        "attempt_id": "attempt-two",
        "origin": "explicit",
        "session_incarnation": "worker-gone-i-incarnation",
        "reserved_revision": 4,
        "state": "reconciled",
        "attempt_count": 1,
        "result": "Submit recovery reconciled",
        "attempted_at": OLD,
        "updated_at": OLD
    });
    quarantined["worker_quarantine"] = json!({
        "schema_version": "main-agent.worker-quarantine.v1",
        "worker": session_ref("worker-gone-i"),
        "reason": "Fixture worker quarantine",
        "runtime_identity_digest": DIGEST,
        "created_at": OLD
    });
    add_assignment(quarantined);
    add_assignment(assignment(
        "worker-receipt-working",
        "run-worker-receipt",
        "main-gone-12",
        "working",
        Some("worker-gone-j"),
    ));
    let mut recently_touched = assignment(
        "assignment-recent-working",
        "run-assignment-recent",
        "main-gone-13",
        "working",
        Some("worker-gone-k"),
    );
    recently_touched["updated_at"] = json!(RECENT);
    add_assignment(recently_touched);
    add_assignment(assignment(
        "bad-time-working",
        "run-bad-time",
        "main-gone-14",
        "working",
        Some("worker-gone-l"),
    ));
    add_assignment(assignment(
        "receipt-submitted",
        "run-receipt",
        "main-gone-6",
        "submitted",
        Some("worker-gone-f"),
    ));
    add_assignment(assignment(
        "recent-working",
        "run-recent",
        "main-gone-7",
        "working",
        Some("worker-gone-g"),
    ));
    let registry = json!({
        "schema_version": "agent-session.orchestration-registry.v3",
        "runs": runs,
        "assignments": assignments,
        "receipts": {
            "main-gone-6:main-gone-6-incarnation:closeout-0001": {
                "principal_session_id": "main-gone-6",
                "principal_incarnation": "main-gone-6-incarnation",
                "operation": "closeout",
                "request_digest": "0".repeat(64),
                "outcome": {
                    "schema_version": "main-agent.closeout-progress.v1",
                    "state": "in_progress",
                    "run_id": "run-receipt"
                },
                "created_at_epoch": 1
            },
            "worker-gone-j:worker-gone-j-incarnation:delete-0001": {
                "principal_session_id": "worker-gone-j",
                "principal_incarnation": "worker-gone-j-incarnation",
                "operation": "worker-delete",
                "request_digest": "1".repeat(64),
                "outcome": { "schema_version": "main-agent.worker-delete-pending.v1" },
                "created_at_epoch": 1
            }
        }
    });
    private_json(&state_dir.join("orchestration/registry.json"), &registry);
    // An unreadable file-backed operation fence is as uncertain as a valid one.
    let canary_fences = state_dir.join("orchestration/provider-stop-canary-reservations");
    private_dir(&canary_fences);
    private_json(
        &canary_fences.join(
            Sha256::digest(b"canary-working")
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        ),
        &json!({ "schema_version": "fixture-unreadable-reservation" }),
    );
    private_json(
        &state_dir.join("coordination/registry.json"),
        &json!({
            "schema_version": "agent-session.coordination-registry.v2",
            "claims": [claim("main-claimed"), claim("worker-claimed")]
        }),
    );
}

fn registry(state_dir: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(state_dir.join("orchestration/registry.json")).expect("registry"),
    )
    .expect("registry json")
}

fn refusal_codes(data: &Value) -> Vec<(String, String)> {
    let mut codes = data["refused"]
        .as_array()
        .expect("refused runs")
        .iter()
        .map(|refused| {
            (
                refused["run_id"].as_str().unwrap().to_string(),
                refused["code"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    codes.sort();
    codes
}

fn run_ids(values: &Value) -> Vec<String> {
    values
        .as_array()
        .expect("runs")
        .iter()
        .map(|run| run["run_id"].as_str().unwrap().to_string())
        .collect()
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(run, code)| (run.to_string(), code.to_string()))
        .collect()
}

#[test]
fn runs_orphaned_lists_only_runs_without_any_live_owner() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed(&state_dir);
    let before = fs::read(state_dir.join("orchestration/registry.json")).expect("registry");

    let listed = main_agent(&state_dir, &["runs", "orphaned", "--format", "json"]);
    assert_eq!(listed.code, 0, "{}", listed.json);
    let data = &listed.json["data"];
    assert_eq!(data["schema_version"], "main-agent.orphaned-runs.v1");
    assert_eq!(
        run_ids(&data["orphaned"]),
        ["run-assignment-recent", "run-orphan", "run-recent"]
    );
    assert_eq!(
        refusal_codes(data),
        owned(&[
            ("run-bad-time", "orphaned-run-too-recent"),
            ("run-canary-fence", "orphaned-run-operation-pending"),
            ("run-cleanup-pending", "orphaned-run-worker-live"),
            (
                "run-controller-claim",
                "orphaned-run-controller-claim-active"
            ),
            ("run-launch-pending", "orphaned-run-worker-live"),
            ("run-operation", "orphaned-run-operation-pending"),
            ("run-quarantined", "orphaned-run-operation-pending"),
            ("run-receipt", "orphaned-run-operation-pending"),
            ("run-worker-claim", "orphaned-run-worker-live"),
            ("run-worker-receipt", "orphaned-run-operation-pending"),
            ("run-worker-session", "orphaned-run-worker-live"),
        ])
    );
    let blockers = |run_id: &str| {
        data["refused"]
            .as_array()
            .unwrap()
            .iter()
            .find(|refused| refused["run_id"] == run_id)
            .map(|refused| refused["blockers"].clone())
            .unwrap_or_else(|| panic!("{run_id} was not refused"))
    };
    assert_eq!(
        blockers("run-quarantined")[0]["operation"],
        "worker-quarantined"
    );
    assert_eq!(
        blockers("run-worker-receipt")[0],
        json!({
            "code": "orphaned-run-operation-pending",
            "session_id": "worker-gone-j",
            "operation": "worker-delete"
        })
    );
    assert_eq!(
        blockers("run-canary-fence")[0]["assignment_id"],
        "canary-working"
    );
    assert_eq!(
        blockers("run-bad-time")[0]["reason"],
        "activity-time-unparseable",
        "an unparseable activity time never ages in, even with no bound"
    );
    assert_eq!(data["active_runs"], 15);
    assert_eq!(data["controller_live_runs"], 1);
    let orphan = data["orphaned"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == "run-orphan")
        .expect("run-orphan is orphaned");
    assert_eq!(orphan["from_revision"], 3);
    assert_eq!(orphan["to_state"], "closed");
    assert_eq!(orphan["reason"], "orphaned");
    assert_eq!(
        orphan["assignments"],
        json!([
            {
                "assignment_id": "orphan-accepted",
                "from_state": "accepted",
                "to_state": "released",
                "from_revision": 5,
                "to_revision": 6,
                "reason": "orphaned-run"
            },
            {
                "assignment_id": "orphan-working",
                "from_state": "working",
                "to_state": "cancelled",
                "from_revision": 5,
                "to_revision": 6,
                "reason": "orphaned-run"
            }
        ])
    );
    assert_eq!(
        fs::read(state_dir.join("orchestration/registry.json")).expect("registry"),
        before,
        "listing is read-only"
    );
}

#[test]
fn close_orphaned_defaults_to_a_dry_run_and_applies_only_the_reviewed_plan() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed(&state_dir);
    let before = fs::read(state_dir.join("orchestration/registry.json")).expect("registry");

    let dry_run = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "7d",
            "--format",
            "json",
        ],
    );
    assert_eq!(dry_run.code, 0, "{}", dry_run.json);
    let plan = &dry_run.json["data"];
    assert_eq!(plan["schema_version"], "main-agent.close-orphaned-runs.v1");
    assert_eq!(plan["mode"], "dry-run");
    assert_eq!(plan["applied"], false);
    assert_eq!(plan["older_than_seconds"], 7 * 24 * 60 * 60);
    assert_eq!(run_ids(&plan["runs"]), ["run-orphan"]);
    for recent in ["run-recent", "run-assignment-recent", "run-bad-time"] {
        assert!(
            refusal_codes(plan)
                .contains(&(recent.to_string(), "orphaned-run-too-recent".to_string())),
            "{recent}: {plan}"
        );
    }
    let recent_activity = plan["refused"]
        .as_array()
        .unwrap()
        .iter()
        .find(|refused| refused["run_id"] == "run-assignment-recent")
        .expect("recently touched run");
    assert_eq!(
        recent_activity["last_activity_at"], RECENT,
        "the newest assignment activity keeps the run too recent"
    );
    let digest = plan["plan_digest"]
        .as_str()
        .expect("plan digest")
        .to_string();
    assert!(digest.starts_with("sha256:"), "{digest}");
    assert_eq!(
        fs::read(state_dir.join("orchestration/registry.json")).expect("registry"),
        before,
        "the default dry run changes nothing"
    );

    let stale = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "7d",
            "--apply",
            "--if-plan-digest",
            DIGEST,
            "--idempotency-key",
            "close-orphaned-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(stale.code, 65, "{}", stale.json);
    assert_eq!(stale.json["error"]["code"], "orphaned-run-plan-conflict");
    assert_eq!(
        stale.json["error"]["details"]["current_plan_digest"],
        digest.as_str()
    );
    assert_eq!(
        fs::read(state_dir.join("orchestration/registry.json")).expect("registry"),
        before,
        "a stale plan changes nothing"
    );

    let apply_args = [
        "runs",
        "close-orphaned",
        "--older-than",
        "7d",
        "--apply",
        "--if-plan-digest",
        digest.as_str(),
        "--idempotency-key",
        "close-orphaned-0002",
        "--format",
        "json",
    ];
    let applied = main_agent(&state_dir, &apply_args);
    assert_eq!(applied.code, 0, "{}", applied.json);
    let result = &applied.json["data"];
    assert_eq!(result["mode"], "apply");
    assert_eq!(result["applied"], true);
    assert_eq!(result["plan_digest"], digest.as_str());
    assert_eq!(result["runs"], plan["runs"]);

    let after = registry(&state_dir);
    let orphan = &after["runs"]["run-orphan"];
    assert_eq!(orphan["state"], "closed");
    assert_eq!(orphan["revision"], 4);
    let states = |id: &str| {
        let assignment = &after["assignments"][id];
        (
            assignment["state"].as_str().unwrap().to_string(),
            assignment["revision"].as_u64().unwrap(),
        )
    };
    assert_eq!(states("orphan-working"), ("cancelled".to_string(), 6));
    assert_eq!(states("orphan-accepted"), ("released".to_string(), 6));
    assert_eq!(
        states("orphan-cancelled"),
        ("cancelled".to_string(), 5),
        "an already terminal assignment is left as it was"
    );
    assert!(
        after["assignments"]["orphan-working"]["blocker_summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("orphaned-run")),
        "{}",
        after["assignments"]["orphan-working"]
    );
    for untouched in [
        "run-controller-live",
        "run-controller-claim",
        "run-worker-session",
        "run-worker-claim",
        "run-launch-pending",
        "run-operation",
        "run-receipt",
        "run-recent",
        "run-cleanup-pending",
        "run-canary-fence",
        "run-quarantined",
        "run-worker-receipt",
        "run-assignment-recent",
        "run-bad-time",
    ] {
        assert_eq!(after["runs"][untouched]["state"], "active", "{untouched}");
        assert_eq!(after["runs"][untouched]["revision"], 3, "{untouched}");
    }
    for untouched in [
        "controller-live-working",
        "worker-session-blocked",
        "worker-claim-working",
        "launch-pending-starting",
        "operation-working",
        "receipt-submitted",
        "recent-working",
        "cleanup-pending-released",
        "canary-working",
        "quarantined-working",
        "worker-receipt-working",
        "assignment-recent-working",
        "bad-time-working",
    ] {
        assert_eq!(
            after["assignments"][untouched]["revision"], 5,
            "{untouched}"
        );
    }
    assert!(
        state_dir.join("sessions/worker-live-session").is_dir()
            && state_dir.join("sessions/worker-cleanup-pending").is_dir()
            && state_dir.join("sessions/main-live").is_dir(),
        "terminalization never deletes sessions"
    );

    let replay = main_agent(&state_dir, &apply_args);
    assert_eq!(replay.code, 0, "{}", replay.json);
    assert_eq!(replay.json["data"], applied.json["data"]);
    assert_eq!(
        registry(&state_dir),
        after,
        "replaying the same key is a no-op"
    );

    let conflicting = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "30d",
            "--apply",
            "--if-plan-digest",
            digest.as_str(),
            "--idempotency-key",
            "close-orphaned-0002",
            "--format",
            "json",
        ],
    );
    assert_eq!(conflicting.code, 65, "{}", conflicting.json);
    assert_eq!(conflicting.json["error"]["code"], "idempotency-conflict");

    let listed = main_agent(&state_dir, &["runs", "orphaned", "--format", "json"]);
    assert_eq!(
        run_ids(&listed.json["data"]["orphaned"]),
        ["run-assignment-recent", "run-recent"]
    );
}

#[test]
fn close_orphaned_refuses_a_plan_whose_records_changed_after_review() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed(&state_dir);
    let dry_run = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "7d",
            "--format",
            "json",
        ],
    );
    assert_eq!(dry_run.code, 0, "{}", dry_run.json);
    let reviewed = dry_run.json["data"]["plan_digest"]
        .as_str()
        .expect("plan digest")
        .to_string();

    // Another writer advances a planned assignment between review and apply.
    let path = state_dir.join("orchestration/registry.json");
    let mut changed = registry(&state_dir);
    changed["assignments"]["orphan-working"]["revision"] = json!(6);
    private_json(&path, &changed);
    let before = fs::read(&path).expect("registry");

    let applied = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "7d",
            "--apply",
            "--if-plan-digest",
            reviewed.as_str(),
            "--idempotency-key",
            "close-orphaned-stale",
            "--format",
            "json",
        ],
    );
    assert_eq!(applied.code, 65, "{}", applied.json);
    assert_eq!(applied.json["error"]["code"], "orphaned-run-plan-conflict");
    let current = applied.json["error"]["details"]["current_plan_digest"]
        .as_str()
        .expect("current digest");
    assert_ne!(current, reviewed);
    assert_eq!(fs::read(&path).expect("registry"), before);
}

#[test]
fn close_orphaned_apply_requires_a_reviewed_plan_and_key() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed(&state_dir);
    let unfenced = main_agent(
        &state_dir,
        &[
            "runs",
            "close-orphaned",
            "--older-than",
            "7d",
            "--apply",
            "--format",
            "json",
        ],
    );
    assert_eq!(unfenced.code, 64, "{}", unfenced.json);
    assert_eq!(unfenced.json["error"]["code"], "parse-error");
    let unbounded = main_agent(&state_dir, &["runs", "close-orphaned", "--format", "json"]);
    assert_eq!(unbounded.code, 64, "{}", unbounded.json);
    assert_eq!(unbounded.json["error"]["code"], "parse-error");
}
