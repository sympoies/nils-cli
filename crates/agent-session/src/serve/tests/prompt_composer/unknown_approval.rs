//! Pending approval remains input authority when structured idle is uncertain.
use super::super::*;
use pretty_assertions::assert_eq;

fn pending_approval_with_fresh_idle(state: &Arc<ServeState>) -> String {
    let id = "blocked";
    let record = load_session_record(&state.context, id).unwrap();
    block_on_provider_attention(state, id, "claude");
    let controller = crate::activity::controller_state_for_record(&state.context, &record).unwrap();
    let runtime = record.runtime.as_ref().unwrap();
    let idle = json!({
        "schema_version": "agent-session.activity-shadow.v1",
        "runtime_id": runtime.launch_id,
        "runtime_generation": runtime.generation,
        "activity_revision": controller.revision,
        "observation": {
            "observer_version": "claude-agents.v1",
            "rule_id": "claude_agents_idle",
            "observed_at": jiff::Timestamp::now().to_string(),
            "projection": "waiting",
            "disagrees": true
        }
    });
    fs::write(
        crate::session_dir(&state.context, id).join("activity.shadow.json"),
        serde_json::to_vec(&idle).unwrap(),
    )
    .unwrap();
    let view = crate::activity::state_for_view(&state.context, &record).unwrap();
    assert_eq!(view.phase, crate::activity::TurnPhase::Unknown);
    assert_eq!(
        view.diagnostic.unwrap().reason,
        "claude_turn_outcome_unknown"
    );
    assert_eq!(controller.phase, crate::activity::TurnPhase::NeedsInput);
    assert!(controller.current_turn.unwrap().attention.is_some());
    runtime.launch_id.clone()
}

#[tokio::test]
async fn send_refuses_pending_approval_with_unknown_idle_view() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = super::recording::tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let state = state(tmp.path(), Some(TOKEN), tmux);
    pending_approval_with_fresh_idle(&state);

    let (status, body) = call(
        router(state),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({ "text": "please continue", "keys": ["enter"] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "claude-permission-hook-required");
    assert!(tmux_never_ran(&journal));
}

#[tokio::test]
async fn structured_prompt_refuses_pending_approval_with_unknown_idle_view() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let state = state(tmp.path(), Some(TOKEN), tmux);
    pending_approval_with_fresh_idle(&state);

    let (status, body) = call(
        router(state),
        post_json(
            "/sessions/blocked/prompt",
            Some(TOKEN),
            json!({ "text": "please continue" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "agent-blocked");
    assert!(tmux_never_ran(&journal));
}

#[test]
fn pane_prompt_refuses_pending_approval_with_unknown_idle_view() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let state = state(tmp.path(), Some(TOKEN), tmux.clone());
    let launch_id = pending_approval_with_fresh_idle(&state);

    let error = crate::prompt_delivery::deliver_claude_structured_prompt_locked(
        &state.context,
        "blocked",
        &launch_id,
        "please continue",
        &tmux,
    )
    .err()
    .expect("pending approval must refuse pane delivery");

    assert_eq!(error.code(), "agent-blocked");
    assert!(!tmux_wrote_to_the_pane(&journal));
}

#[tokio::test]
async fn title_update_preserves_pending_approval_with_unknown_idle_view() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let state = state(tmp.path(), Some(TOKEN), tmux);
    pending_approval_with_fresh_idle(&state);

    let (status, body) = call(
        router(state.clone()),
        patch_json(
            "/sessions/blocked",
            Some(TOKEN),
            json!({ "title": "renamed while blocked" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(!tmux_wrote_to_the_pane(&journal));
    assert_eq!(
        load_session_record(&state.context, "blocked")
            .unwrap()
            .title
            .as_deref(),
        Some("renamed while blocked")
    );
}
