use super::*;

pub(crate) fn attention_authority(record: &SessionRecord) -> &'static str {
    if record.runtime.as_ref().is_some_and(|runtime| {
        runtime.kind == RUNTIME_KIND
            && runtime
                .extra
                .get(ATTENTION_AUTHORITY_KEY)
                .and_then(Value::as_str)
                == Some(ATTENTION_AUTHORITY_PROTOCOL)
    }) {
        ATTENTION_AUTHORITY_PROTOCOL
    } else {
        ATTENTION_AUTHORITY_HOOK
    }
}

/// Only protocol versions whose metadata contract has been checked explicitly.
pub(crate) fn exact_attention_version_is_audited(version: (u64, u64, u64)) -> bool {
    matches!(version, (0, 144, 1) | (0, 144, 3) | (0, 162, 1))
}

pub(super) fn nonblocking_input(value: &Value) -> Option<ServerProjection> {
    if value.get("method").and_then(Value::as_str) != Some("item/tool/requestUserInput") {
        return None;
    }
    match value.pointer("/params/isBlocking") {
        Some(Value::Bool(false)) => Some(ServerProjection::Irrelevant),
        Some(Value::Bool(true)) | None => None, // Older audited versions omit this field.
        Some(_) => Some(ServerProjection::RejectedUnique),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn nonblocking_user_input_does_not_request_human_attention() {
        let raw = json!({"id": 17, "method": "item/tool/requestUserInput", "params": {
            "threadId": "thread-a", "turnId": "turn-a", "itemId": "item-a",
            "isBlocking": false, "questions": [{"question": "private-question-canary"}]
        }});
        assert!(matches!(
            server_observation(&raw),
            ServerProjection::Irrelevant
        ));
    }

    #[test]
    fn blocking_input_is_projected_without_question_content() {
        let raw = json!({"id": 17, "method": "item/tool/requestUserInput", "params": {
            "threadId": "thread-a", "turnId": "turn-a", "itemId": "item-a",
            "isBlocking": true, "questions": [{"question": "private-question-canary"}]
        }});
        let ServerProjection::Unique(value) = server_observation(&raw) else {
            panic!("blocking request must project");
        };
        assert!(!value.to_string().contains("private-question-canary"));
        assert!(value["params"]["kind"] == "clarification");
    }

    #[test]
    fn malformed_blocking_flag_is_not_admitted_as_attention() {
        let raw = json!({"id": 17, "method": "item/tool/requestUserInput", "params": {
            "threadId": "thread-a", "turnId": "turn-a", "isBlocking": "private-invalid-canary"
        }});
        assert!(matches!(
            server_observation(&raw),
            ServerProjection::RejectedUnique
        ));
    }

    #[tokio::test]
    async fn current_protocol_questions_and_approvals_remain_pending_until_exactly_resolved() {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = super::super::tests::record_with_runtime(
            "attention-fixture",
            &tmp.path().join("server.sock"),
        );
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        let runtime_id = &record.runtime.as_ref().unwrap().launch_id;
        let input = |kind: &str, event_id: &str| {
            serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION, "event_id": event_id,
            "runtime_id": runtime_id, "provider": "codex", "provider_session_id": "thread-a",
            "provider_turn_id": "turn-a", "kind": kind, "confidence": "observed", "source_kind": "provider_hook"
        })).unwrap()
        };
        crate::activity::ingest_event(&context, &record.id, input("turn_started", "start"))
            .unwrap();
        let mut reducer = FailureReducer::new("thread-a");
        let mut pending = BTreeMap::new();
        let status = || {
            serde_json::to_value(
                crate::activity::activity_status(&context, &record.id)
                    .unwrap()
                    .turn_state,
            )
            .unwrap()
        };
        for (id, method) in [
            (json!(1), "item/tool/requestUserInput"),
            (json!("1"), "item/commandExecution/requestApproval"),
        ] {
            let raw = json!({"id": id, "method": method, "params": {"threadId": "thread-a", "turnId": "turn-a", "itemId": "item-a", "isBlocking": true, "questions": [{"question": "private-question-canary"}], "command": "private-command-canary"}});
            let ServerProjection::Unique(value) = server_observation(&raw) else {
                panic!("exact request");
            };
            process_live_message(&context, &record, &mut reducer, Some(&mut pending), &value)
                .await
                .unwrap();
        }
        assert_eq!(status()["current_turn"]["attention"]["pending_count"], 2);
        crate::activity::ingest_event(&context, &record.id, input("progress", "parallel-progress"))
            .unwrap();
        assert_eq!(status()["phase"], "needs_input");
        for (request, remaining) in [(json!("unmatched"), 2), (json!(1), 1), (json!("1"), 0)] {
            let ServerProjection::Unique(value) = server_observation(
                &json!({"method": "serverRequest/resolved", "params": {"threadId": "thread-a", "requestId": request}}),
            ) else {
                panic!("exact resolution");
            };
            process_live_message(&context, &record, &mut reducer, Some(&mut pending), &value)
                .await
                .unwrap();
            let state = status();
            assert_eq!(
                state["phase"],
                if remaining == 0 {
                    "working"
                } else {
                    "needs_input"
                }
            );
            if remaining > 0 {
                assert_eq!(
                    state["current_turn"]["attention"]["pending_count"],
                    remaining
                );
            }
            assert!(!state.to_string().contains("private-question-canary"));
            assert!(!state.to_string().contains("private-command-canary"));
        }
    }
}
