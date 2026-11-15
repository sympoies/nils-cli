//! Guard admission for a stopped turn whose provider completion never arrives.
use super::*;
use pretty_assertions::assert_eq;

fn stopped_fixture() -> Fixture {
    let fixture = Fixture::new("claude");
    // Capture/identity observations in these cases need only a single ASCII
    // composer row. Keep Python startup out of the production capture budget.
    fs::rename(
        fixture.root().join("tmux"),
        fixture.root().join("tmux-default"),
    )
    .unwrap();
    fs::write(
        fixture.root().join("tmux"),
        include_str!("completion-tmux.sh"),
    )
    .unwrap();
    fs::set_permissions(
        fixture.root().join("tmux"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let state_dir = fixture.root().join("state");
    let options = CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_");
    for kind in ["turn_started", "progress", "stop_observed"] {
        let mut event = json!({
            "schema_version": "agent-session.turn-event.v1", "event_id": kind,
            "runtime_id": "runtime-one", "provider": "claude",
            "provider_session_id": "provider-one", "provider_turn_id": "stopped-turn",
            "kind": kind, "source_kind": "provider_hook", "confidence": "observed"
        });
        if kind == "stop_observed" {
            event["completion_candidate_reason"] = json!("background_tasks_present");
        }
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "activity",
                "event",
                "guarded",
                "--stdin",
                "--format",
                "json",
            ],
            &options.clone().with_stdin_str(&event.to_string()),
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
    }
    let dir = state_dir.join("sessions/guarded");
    let path = dir.join("activity.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["state"]["phase"], "working");
    assert_eq!(
        document["state"]["diagnostic"]["reason"],
        "completion_evidence_pending"
    );
    // Cross the quiet window without sleeping; no completion is supplied.
    document["state"]["current_turn"]["started_at"] = json!("2000-01-01T00:00:00Z");
    document["state"]["current_turn"]["last_progress_at"] = json!("2000-01-01T00:01:00Z");
    document["state"]["semantic_event"]["observed_at"] = json!("2000-01-01T00:02:00Z");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let status = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state_dir.to_str().unwrap(),
            "activity",
            "status",
            "guarded",
            "--format",
            "json",
        ],
        &options,
    );
    assert_eq!(status.code, 0, "{}", status.stderr_text());
    let state = &status.stdout_json()["data"]["turn_state"];
    assert_eq!(state["phase"], "unknown");
    assert_eq!(state["source"]["confidence"], "inferred");
    assert_eq!(state["diagnostic"]["reason"], "completion_evidence_missing");
    assert_eq!(
        state["diagnostic"]["completion_candidate_reason"],
        "background_tasks_present"
    );

    fixture
}

#[test]
fn missing_completion_refuses_with_actionable_reason_before_paste() {
    let fixture = stopped_fixture();
    let dir = fixture.root().join("state/sessions/guarded");
    let path = dir.join("activity.json");
    let journal_path = dir.join("activity.journal.jsonl");
    let before = fs::read(&path).unwrap();
    let journal = fs::read(&journal_path).unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    let refusal = fixture.refused(&output, "send-completion-evidence-missing");
    assert_eq!(refusal["error"]["details"]["guard"], "idle");
    assert_eq!(refusal["error"]["details"]["stage"], "admission");
    let message = refusal["error"]["message"].as_str().unwrap();
    assert!(message.contains("activity status"));
    assert!(message.contains("provider completion"));
    assert!(message.contains("runtime-bound provider idle evidence"));
    assert_eq!(refusal["error"]["details"]["safe_to_retry"], true);
    fixture.no_delivery();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&journal_path).unwrap(), journal);
}

#[test]
fn stopped_guarded_wake_recovers_on_fresh_structured_idle_without_replaying() {
    let fixture = stopped_fixture();
    let guards = [
        "--require-idle",
        "--require-empty-composer",
        "--idempotency-key",
        "stopped-wake",
    ];
    fixture.refused(&fixture.send(&guards), "send-completion-evidence-missing");
    fixture.no_delivery();
    write_idle(&fixture, idle_observation(&fixture));
    let dir = fixture.root().join("state/sessions/guarded");
    let snapshot = fs::read(dir.join("activity.json")).unwrap();
    let journal = fs::read(dir.join("activity.journal.jsonl")).unwrap();
    let recovered = fixture.send(&guards);
    assert_eq!(recovered.code, 0, "{}", recovered.stdout_text());
    assert_eq!(
        recovered.stdout_json()["data"]["submission"]["outcome"],
        "submitted"
    );
    assert_eq!(fs::read(dir.join("activity.json")).unwrap(), snapshot);
    assert_eq!(
        fs::read(dir.join("activity.journal.jsonl")).unwrap(),
        journal
    );
    let submissions = fs::read(fixture.root().join("submissions")).unwrap();
    fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
    assert_eq!(
        fs::read(fixture.root().join("submissions")).unwrap(),
        submissions
    );
}

fn idle_observation(fixture: &Fixture) -> Value {
    let dir = fixture.root().join("state/sessions/guarded");
    let document: Value =
        serde_json::from_slice(&fs::read(dir.join("activity.json")).unwrap()).unwrap();
    json!({
        "schema_version": "agent-session.activity-shadow.v1",
        "runtime_id": "runtime-one", "runtime_generation": 1,
        "activity_revision": document["state"]["revision"],
        "observation": {"observer_version": "claude-agents.v1", "rule_id": "claude_agents_idle",
            "observed_at": jiff::Timestamp::now().to_string(), "projection": "waiting", "disagrees": true}
    })
}

fn write_idle(fixture: &Fixture, observation: Value) {
    fs::write(
        fixture
            .root()
            .join("state/sessions/guarded/activity.shadow.json"),
        serde_json::to_vec(&observation).unwrap(),
    )
    .unwrap();
}

#[test]
fn stopped_recovery_rejects_unbound_or_nonidle_evidence() {
    for case in [
        "runtime",
        "generation",
        "revision",
        "future",
        "busy",
        "blocked",
        "screen",
    ] {
        let fixture = stopped_fixture();
        let mut idle = idle_observation(&fixture);
        match case {
            "runtime" => idle["runtime_id"] = json!("replaced"),
            "generation" => idle["runtime_generation"] = json!(2),
            "revision" => idle["activity_revision"] = json!(0),
            "future" => idle["observation"]["observed_at"] = json!("2100-01-01T00:00:00Z"),
            "busy" => {
                idle["observation"]["rule_id"] = json!("claude_agents_busy");
                idle["observation"]["projection"] = json!("working");
            }
            "blocked" => {
                idle["observation"]["rule_id"] = json!("claude_agents_waiting_permission_prompt");
                idle["observation"]["projection"] = json!("needs_input");
            }
            "screen" => idle["observation"]["observer_version"] = json!("terminal-shadow.v1"),
            _ => unreachable!(),
        }
        write_idle(&fixture, idle);
        let refusal = fixture.refused(
            &fixture.send(&["--require-idle", "--require-empty-composer"]),
            "send-completion-evidence-missing",
        );
        assert_eq!(refusal["error"]["details"]["safe_to_retry"], true, "{case}");
        fixture.no_delivery();
    }
}

#[test]
fn stopped_recovery_preserves_drafts_identity_and_uncertain_claims() {
    for (flag, code) in [
        ("composer", "send-composer-not-empty"),
        ("runtime-replaced", "send-identity-mismatch"),
        ("replace-after-paste", "send-composer-mismatch"),
    ] {
        let fixture = stopped_fixture();
        write_idle(&fixture, idle_observation(&fixture));
        fs::write(fixture.root().join(flag), "owner draft").unwrap();
        let guards = [
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "one-wake",
        ];
        let refusal = fixture.refused(&fixture.send(&guards), code);
        assert_eq!(
            refusal["error"]["details"]["safe_to_retry"],
            flag != "replace-after-paste"
        );
        if flag == "replace-after-paste" {
            write_idle(&fixture, idle_observation(&fixture));
            fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
            assert!(!fixture.root().join("submissions").exists());
            assert_eq!(
                fs::read_to_string(fixture.root().join("composer")).unwrap(),
                "replacement draft"
            );
        } else {
            fixture.no_delivery();
        }
    }
}

#[test]
fn fresh_idle_cannot_override_new_work_attention_or_a_foreign_stop() {
    for kind in ["progress", "attention_requested", "stop_observed"] {
        let fixture = stopped_fixture();
        let state_dir = fixture.root().join("state");
        let mut event = json!({
            "schema_version": "agent-session.turn-event.v1", "event_id": "later-event",
            "runtime_id": "runtime-one", "provider": "claude",
            "provider_session_id": "provider-one",
            "provider_turn_id": if kind == "stop_observed" { "foreign-turn" } else { "stopped-turn" },
            "kind": kind, "source_kind": "provider_hook", "confidence": "observed"
        });
        if kind == "attention_requested" {
            event["attention_id"] = json!("pending-question");
            event["attention_kind"] = json!("clarification");
        }
        let result = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "activity",
                "event",
                "guarded",
                "--stdin",
                "--format",
                "json",
            ],
            &CmdOptions::new()
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_stdin_str(&event.to_string()),
        );
        assert_eq!(result.code, 0, "{}", result.stdout_text());
        // Even a fresh sample at the new revision cannot override this state.
        write_idle(&fixture, idle_observation(&fixture));
        let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
        assert_eq!(output.code, 1, "{kind}: {}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["details"]["safe_to_retry"],
            true
        );
        fixture.no_delivery();
    }
}
