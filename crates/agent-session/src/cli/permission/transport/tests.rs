//! Final delivery uses accepted sockets and synthetic provider activity only.
use super::*;
use crate::{activity, cli::AgentKind, cli::permission::Decision};
use pretty_assertions::assert_eq;

fn hook(context: &CliContext, name: &str) {
    let mut payload = json!({
        "hook_event_name": name, "session_id": "provider-one", "prompt_id": "turn-one",
        "tool_name": "Bash", "tool_input": {"command": "fixture-operation"},
    });
    if matches!(name, "PreToolUse" | "PostToolUse") {
        payload["tool_use_id"] = json!("call-one");
    }
    activity::ingest_provider_hook_input(
        context,
        AgentKind::Claude,
        None,
        activity::ProviderHookInput {
            id: "guarded",
            runtime_id: "runtime-one",
            payload: &serde_json::to_vec(&payload).unwrap(),
            attention_authority: None,
        },
    )
    .unwrap();
}

fn session() -> (tempfile::TempDir, CliContext, Pending) {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    let record: SessionRecord = serde_json::from_value(json!({
        "schema_version": "agent-session.session.v1", "id": "guarded",
        "agent": "claude", "mode": "interactive", "title": null,
        "cwd": temp.path(), "tmux_session": "modeled-guarded",
        "prompt_file": null, "log_file": null,
        "created_at": "2000-01-01T00:00:00Z", "updated_at": "2000-01-01T00:00:00Z",
        "runtime": { "kind": "tmux", "tmux_session": "modeled-guarded",
            "generation": 1, "launch_id": "runtime-one", "started_at": "2000-01-01T00:00:00Z" },
        "provider_resume": { "provider": "claude", "session_id": "provider-one",
            "captured_at": "2000-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": [] }
    }))
    .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    activity::activate_runtime(&context, &record).unwrap();
    for name in ["UserPromptSubmit", "PreToolUse", "PermissionRequest"] {
        hook(&context, name);
    }
    let document: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(&context, "guarded").join("activity.json")).unwrap(),
    )
    .unwrap();
    let attention_id = document["pending_attention"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let call_id = {
        let _record = crate::acquire_session_record_lock(&context, "guarded").unwrap();
        with_permission_call(&context, &record, &attention_id, |call| Ok(call.to_owned())).unwrap()
    };
    let pending = Pending {
        request_id: uuid::Uuid::new_v4().to_string(),
        call_id,
        runtime_id: "runtime-one".into(),
        attention_id,
        socket: PathBuf::new(),
    };
    (temp, context, pending)
}

fn received_answer(
    pending: &Pending,
    expire_during_read: bool,
) -> (UnixStream, UnixStream, AnswerRequest, Instant) {
    // The socket pair is already accepted before its deadline exists. Complete
    // the valid body only after expiry, without relying on the waiter's polling
    // loop or a race between connect and its pre-accept correlation check.
    let (mut receiver, mut caller) = UnixStream::pair().unwrap();
    let deadline = Instant::now()
        + if expire_during_read {
            Duration::from_millis(20)
        } else {
            Duration::from_secs(30)
        };
    let request = AnswerRequest {
        request_id: pending.request_id.clone(),
        call_id: pending.call_id.clone(),
        decision: Decision::Allow,
    };
    let bytes = serde_json::to_vec(&request).unwrap();
    caller.write_all(&bytes[..1]).unwrap();
    if expire_during_read {
        thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        assert!(Instant::now() >= deadline);
    }
    caller.write_all(&bytes[1..]).unwrap();
    caller.shutdown(std::net::Shutdown::Write).unwrap();
    let received = serde_json::from_reader((&mut receiver).take(4097)).unwrap();
    (receiver, caller, received, deadline)
}

fn assert_no_ack(mut caller: UnixStream) {
    let mut acknowledgement = Vec::new();
    caller.read_to_end(&mut acknowledgement).unwrap();
    assert!(
        acknowledgement.is_empty(),
        "refused answer was acknowledged"
    );
}

#[test]
fn accepted_permission_answer_completed_after_expiry_emits_nothing() {
    let (_temp, context, pending) = session();
    let (mut receiver, caller, request, deadline) = received_answer(&pending, true);
    let mut output = Vec::new();
    let result = deliver(
        &context,
        "guarded",
        &pending,
        &request,
        deadline,
        &mut receiver,
        &mut output,
    );
    assert!(output.is_empty(), "expired hook emitted a decision");
    assert_eq!(result.unwrap_err().code(), "permission-hook-expired");
    drop(receiver);
    assert_no_ack(caller);
}

#[test]
fn resolved_permission_after_answer_read_emits_nothing() {
    let (_temp, context, pending) = session();
    let (mut receiver, caller, request, deadline) = received_answer(&pending, false);
    // This drives the actual final emission function after the complete answer
    // is read. Removing its correlation fence must fail this assertion, even if
    // the outer waiter would have noticed resolution before accepting a socket.
    hook(&context, "PostToolUse");
    let mut output = Vec::new();
    let result = deliver(
        &context,
        "guarded",
        &pending,
        &request,
        deadline,
        &mut receiver,
        &mut output,
    );
    assert!(output.is_empty(), "resolved hook emitted a decision");
    assert_eq!(
        result.unwrap_err().code(),
        "permission-request-uncorrelated"
    );
    drop(receiver);
    assert_no_ack(caller);
}
