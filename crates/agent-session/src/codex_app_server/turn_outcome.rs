use super::*;

/// Feeds a live turn outcome into the activity reducer: a successful
/// completion closes the bound open turn, a structured failure records why.
pub(super) async fn ingest(
    context: &CliContext,
    record: &SessionRecord,
    reducer: &mut FailureReducer,
    value: &Value,
) -> Result<(), String> {
    spawn_completion(context, record, reducer, value)?;
    let Some(failure) = reducer.ingest(value) else {
        return Ok(());
    };
    let context = context.clone();
    let id = record.id.clone();
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    tokio::task::spawn_blocking(move || {
        crate::activity::ingest_codex_app_server_failure_with_kind(
            &context,
            &id,
            &runtime_id,
            &failure.thread_id,
            &failure.turn_id,
            failure.kind,
        )
    })
    .await
    .map_err(|_| "Codex failure ingestion worker failed".to_string())?
    .map_err(|err| format!("Codex failure ingestion failed: {}", err.code()))?;
    Ok(())
}

/// Completion evidence is fail-open and never stalls the live reader: only a
/// completion for the turn the activity view shows open is ingested, the
/// bounded activity-lock wait runs off the observation path, and the activity
/// reducer re-checks the exact open turn under its lock.
fn spawn_completion(
    context: &CliContext,
    record: &SessionRecord,
    reducer: &FailureReducer,
    value: &Value,
) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    let Some(turn_id) = successful_completion(reducer, value) else {
        return Ok(None);
    };
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    let open_turn = crate::activity::activity_status_for_record(context, record)
        .ok()
        .and_then(|status| status.turn_state.current_turn)
        .and_then(|turn| turn.provider_turn_id);
    if open_turn.is_none()
        || open_turn != crate::activity::projected_codex_turn_identifier(&runtime_id, turn_id).ok()
    {
        return Ok(None);
    }
    let context = context.clone();
    let id = record.id.clone();
    let thread_id = reducer.thread_id.clone();
    let turn_id = turn_id.to_string();
    Ok(Some(tokio::task::spawn_blocking(move || {
        if crate::activity::ingest_codex_app_server_completion(
            &context,
            &id,
            &runtime_id,
            &thread_id,
            &turn_id,
        )
        .is_err()
        {
            eprintln!("warning: Codex turn completion reporting degraded");
        }
    })))
}

/// The turn id of a successful `turn/completed` on this reducer's thread.
fn successful_completion<'a>(reducer: &FailureReducer, message: &'a Value) -> Option<&'a str> {
    if message.get("method").and_then(Value::as_str) != Some("turn/completed")
        || message.pointer("/params/threadId").and_then(Value::as_str)
            != Some(reducer.thread_id.as_str())
        || message
            .pointer("/params/turn/status")
            .and_then(Value::as_str)
            != Some("completed")
    {
        return None;
    }
    message
        .pointer("/params/turn/id")
        .and_then(Value::as_str)
        .filter(|turn_id| protocol_id_is_valid(turn_id))
}

#[cfg(test)]
mod tests {
    use super::super::tests::record_with_runtime;
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::time::{Duration, Instant};

    fn codex_turn_event(
        runtime_id: &str,
        kind: &str,
        event_id: &str,
        turn_id: &str,
    ) -> crate::activity::TurnEvent {
        serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": runtime_id,
            "provider": "codex",
            "provider_session_id": "thread-a",
            "provider_turn_id": turn_id,
            "kind": kind,
            "confidence": if kind == "turn_completed" { "authoritative" } else { "observed" },
            "source_kind": "provider_hook"
        }))
        .unwrap()
    }

    fn completed(thread_id: &str, turn_id: &str, turn_status: &str) -> Value {
        let raw = json!({"method":"turn/completed", "params":{"threadId":thread_id,
            "turn":{"id":turn_id, "status":turn_status}}});
        let ServerProjection::Unique(projected) = server_observation(&raw) else {
            panic!("structured turn completion projection");
        };
        projected
    }

    #[tokio::test]
    async fn successful_turn_completed_closes_only_the_bound_open_turn_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record_with_runtime("turn-complete", &tmp.path().join("server.sock"));
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        let runtime_id = record.runtime.as_ref().unwrap().launch_id.clone();
        let start = |event_id: &str, turn_id: &str| {
            crate::activity::ingest_event(
                &context,
                &record.id,
                codex_turn_event(&runtime_id, "turn_started", event_id, turn_id),
            )
            .unwrap();
        };
        let status = || {
            serde_json::to_value(
                crate::activity::activity_status(&context, &record.id)
                    .unwrap()
                    .turn_state,
            )
            .unwrap()
        };
        let mut reducer = FailureReducer::new("thread-a");
        start("start-a", "turn-a");

        // Another thread, an older turn, a non-success status, or a stale
        // runtime generation never closes the open turn.
        let mut stale_record = record.clone();
        stale_record.runtime.as_mut().unwrap().launch_id = "runtime-stale".to_string();
        for (record, message) in [
            (&record, completed("thread-b", "turn-a", "completed")),
            (&record, completed("thread-a", "turn-old", "completed")),
            (&record, completed("thread-a", "turn-a", "interrupted")),
            (&stale_record, completed("thread-a", "turn-a", "completed")),
        ] {
            if let Some(task) = spawn_completion(&context, record, &reducer, &message).unwrap() {
                task.await.unwrap();
            }
            assert_eq!(status()["phase"], "working", "{message}");
        }

        // The live reader admits the exact completion.
        process_live_message(
            &context,
            &record,
            &mut reducer,
            None,
            &completed("thread-a", "turn-a", "completed"),
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while status()["phase"] != "waiting" && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let state = status();
        assert_eq!(state["phase"], "waiting");
        assert_eq!(state["last_turn"]["outcome"], "completed");
        let revision = state["revision"].clone();

        // The notify completion for the same turn and a replayed protocol
        // completion are duplicates.
        let notify = crate::activity::ingest_event(
            &context,
            &record.id,
            codex_turn_event(&runtime_id, "turn_completed", "notify-a", "turn-a"),
        )
        .unwrap();
        assert!(notify.duplicate);
        let replay = completed("thread-a", "turn-a", "completed");
        assert!(
            spawn_completion(&context, &record, &reducer, &replay)
                .unwrap()
                .is_none()
        );
        let protocol_replay = || {
            crate::activity::ingest_codex_app_server_completion(
                &context,
                &record.id,
                &runtime_id,
                "thread-a",
                "turn-a",
            )
            .unwrap()
        };
        assert!(protocol_replay().duplicate);
        assert_eq!(status()["revision"], revision);

        // A late completion for the closed turn never closes a newer turn.
        start("start-b", "turn-b");
        assert!(
            spawn_completion(&context, &record, &reducer, &replay)
                .unwrap()
                .is_none()
        );
        protocol_replay();
        assert_eq!(status()["phase"], "working");

        // A busy activity lock delays the completion instead of dropping it.
        let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
        let task = spawn_completion(
            &context,
            &record,
            &reducer,
            &completed("thread-a", "turn-b", "completed"),
        )
        .unwrap()
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(status()["phase"], "working");
        drop(lock);
        task.await.unwrap();
        assert_eq!(status()["phase"], "waiting");
    }
}
