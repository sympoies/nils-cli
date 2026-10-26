use crate::{CliContext, SessionRecord, activity, codex_app_server};

pub(super) fn fenced_eligible(
    context: &CliContext,
    record: &SessionRecord,
    fence: &super::CoordinationNotificationFence,
) -> bool {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        == Some(fence.candidate.target_incarnation.as_str())
        && codex_app_server::runtime_is_supported(record)
        && activity::state_for_view(context, record)
            .is_some_and(|state| eligible(context, record, &state))
}

pub(super) fn precheck(
    context: &CliContext,
    record: &SessionRecord,
    notification: bool,
) -> Result<(), crate::CliError> {
    if notification
        && activity::state_for_view(context, record).is_some_and(|state| {
            (state.phase == activity::TurnPhase::Unknown
                || activity::codex_notification_maybe_idle(&state))
                && !eligible(context, record, &state)
        })
    {
        return Err(crate::CliError::data(
            "coordination-notification-superseded",
            "notification idle eligibility is unproven",
            None,
        ));
    }
    Ok(())
}

/// A quiet hint never grants input. Unknown candidates must prove the exact
/// bound provider conversation idle before the existing notification fence.
/// Called only on the blocking worker while its session-record lock is held.
pub(super) fn eligible(
    context: &CliContext,
    record: &SessionRecord,
    state: &activity::TurnState,
) -> bool {
    if state.phase == activity::TurnPhase::Waiting {
        return true;
    }
    if !activity::codex_notification_maybe_idle(state)
        || !codex_app_server::runtime_is_supported(record)
    {
        return false;
    }
    let Some(resume) = record
        .provider_resume
        .as_ref()
        .filter(|resume| resume.provider == "codex")
    else {
        return false;
    };
    codex_app_server::probe_idle_conversation(context, record, Some(&resume.session_id))
        .is_ok_and(|thread| thread == resume.session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn quiet_candidate_requires_native_idle_and_never_sends_input() {
        for (candidate, status) in [
            ("starting", "idle"),
            ("starting", "active"),
            ("quiet", "idle"),
            ("quiet", "active"),
        ] {
            let tmp = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            let cwd = tmp.path().join("repo");
            std::fs::create_dir(&cwd).unwrap();
            let mut created = crate::create_record(crate::RecordRequest {
                context: &context,
                agent: crate::cli::AgentKind::Codex,
                mode: "interactive",
                coordination_mode: crate::cli::CoordinationMode::Advisory,
                title: None,
                title_state: None,
                explicit_id: Some("quiet-probe"),
                cwd: &cwd,
                prompt: None,
                log_file_name: None,
                agent_args: vec![],
                agent_bin: None,
                provider_resume: Some(crate::ProviderResume {
                    provider: "codex".into(),
                    session_id: "session-1".into(),
                    captured_at: "2000-01-01T00:00:00Z".into(),
                    capture_method: "test".into(),
                    resume_args: vec![],
                    extra: BTreeMap::new(),
                }),
            })
            .unwrap();
            created.release_lifecycle_lock();
            let (_socket_dir, socket) = nils_test_support::short_socket_path("quiet-probe.sock");
            let runtime = created.record.runtime.as_mut().unwrap();
            runtime.kind = codex_app_server::RUNTIME_KIND.into();
            runtime.extra.insert(
                codex_app_server::PROTOCOL_KEY.into(),
                json!(codex_app_server::PROTOCOL_VERSION),
            );
            for (key, path) in [
                (codex_app_server::SOCKET_KEY, socket.clone()),
                (codex_app_server::PROXY_KEY, tmp.path().join("proxy")),
                (
                    codex_app_server::THREAD_HANDOFF_KEY,
                    tmp.path().join("handoff"),
                ),
                (
                    codex_app_server::THREAD_ATTACHED_KEY,
                    tmp.path().join("attached"),
                ),
            ] {
                runtime.extra.insert(key.into(), json!(path));
            }
            crate::write_session_record(&context, &created.record).unwrap();
            activity::activate_runtime(&context, &created.record).unwrap();
            let state = activity::activity_status(&context, &created.record.id)
                .unwrap()
                .turn_state;
            let mut value = serde_json::to_value(state).unwrap();
            if candidate == "quiet" {
                value["phase"] = json!("working");
                value["source"]["confidence"] = json!("inferred");
                value["diagnostic"] = json!({
                    "reason":"codex_progress_quiet", "quiet_threshold_seconds":300,
                    "quiet_since":"2000-01-01T00:00:00Z", "quiet_signal":"current_turn.last_progress_at"
                });
            }
            let state = serde_json::from_value(value).unwrap();
            assert!(activity::codex_notification_maybe_idle(&state));
            let listener = tokio::net::UnixListener::bind(socket).unwrap();
            let peer = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut methods = Vec::new();
                while let Some(Ok(message)) = ws.next().await {
                    let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    let method = request["method"].as_str().unwrap();
                    if method == "initialized" {
                        continue;
                    }
                    methods.push(method.to_string());
                    let result = match method {
                        "initialize" => json!({}),
                        "thread/loaded/list" => json!({"data":["session-1"],"nextCursor":null}),
                        "thread/read" => {
                            assert_eq!(request["params"]["includeTurns"], false);
                            json!({"thread":{"id":"session-1","status":{"type":status}}})
                        }
                        _ => panic!("unexpected side effect: {method}"),
                    };
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(
                        json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                    if method == "thread/read" {
                        break;
                    }
                }
                methods
            });
            let admitted =
                tokio::task::spawn_blocking(move || eligible(&context, &created.record, &state))
                    .await
                    .unwrap();
            assert_eq!(admitted, status == "idle");
            assert_eq!(
                peer.await.unwrap(),
                ["initialize", "thread/loaded/list", "thread/read"]
            );
        }
    }
}
