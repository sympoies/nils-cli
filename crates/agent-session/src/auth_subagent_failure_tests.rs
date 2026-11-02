//! A subagent's provider failure is not the main session's failure.
//!
//! The provider tags a hook that fires inside a subagent with `agent_id`; the
//! main thread never carries it, even in an `--agent` session.
use super::*;
use pretty_assertions::assert_eq;

fn started_runtime(context: &CliContext, root: &Path) -> (SessionRecord, String) {
    seed_exited_session(context, root);
    let tmux = fake_tmux(root);
    let launch = running_session(context, &tmux);
    session_ready(context, &launch);
    (load_session_record(context, ID).unwrap(), launch)
}

fn stop_failure(subagent: bool) -> Value {
    let mut payload = json!({
        "hook_event_name": "StopFailure",
        "error": "authentication_failed",
        "session_id": CONVERSATION,
        "agent_type": "general-purpose",
    });
    if subagent {
        payload["agent_id"] = json!("synthetic-subagent");
    }
    payload
}

#[test]
fn subagent_auth_failure_leaves_the_main_session_accepting_input() {
    let tmp = tempfile::tempdir().unwrap();
    let context = context(tmp.path());
    let (record, launch) = started_runtime(&context, tmp.path());
    claude_hook(&context, &launch, stop_failure(true));
    assert!(crate::auth_incident::view(&context, &record).is_none());
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_ok());
    assert!(
        crate::activity::state_for_view(&context, &record)
            .unwrap()
            .last_turn
            .is_none_or(|turn| turn.provider_failure_kind().is_none())
    );
}

#[test]
fn main_session_auth_failure_quarantines_with_a_typed_reason_and_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let context = context(tmp.path());
    let (record, launch) = started_runtime(&context, tmp.path());
    claude_hook(&context, &launch, stop_failure(false));
    let error =
        crate::auth_incident::recovery::ensure_input_allowed(&context, &record).unwrap_err();
    assert_eq!(error.code(), "auth-quarantined");
    let details = error.details().expect("typed refusal details");
    assert_eq!(details["reason"], "auth_failed");
    assert_eq!(details["exit"], json!(["restart", "switch_account"]));
    let text = details.to_string();
    assert!(!text.contains(&tmp.path().to_string_lossy().to_string()));
    assert!(!text.contains("authentication_failed"));
}
