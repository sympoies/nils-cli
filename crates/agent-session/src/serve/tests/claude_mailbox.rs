use pretty_assertions::assert_eq;

use super::*;

#[tokio::test]
async fn claude_notification_requires_detached_stop_and_observes_exact_new_prompt() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "claude", "hs-claude-beta");
    let command_log = tmp.path().join("tmux-commands.log");
    let captured_prompt = tmp.path().join("captured-prompt");
    let attached_state = tmp.path().join("attached-state");
    fs::write(&attached_state, b"1\n").expect("attached state");
    let tmux = tmp.path().join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$1" in
  has-session) exit 0 ;;
  display-message)
read value < {}
printf '%s\n' "$value"
exit 0
;;
  load-buffer)
for value in "$@"; do last="$value"; done
cp "$last" {}
exit 0
;;
  paste-buffer|send-keys|delete-buffer) exit 0 ;;
  *) exit 1 ;;
esac
"#,
            shell_words::quote(&command_log.to_string_lossy()),
            shell_words::quote(&attached_state.to_string_lossy()),
            shell_words::quote(&captured_prompt.to_string_lossy())
        ),
    )
    .expect("tmux script");
    let mut permissions = fs::metadata(&tmux).expect("tmux metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&tmux, permissions).expect("tmux permissions");

    let state = state(tmp.path(), Some(TOKEN), tmux.clone());
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let launch_id = beta
        .runtime
        .as_ref()
        .expect("beta runtime")
        .launch_id
        .clone();
    let stop = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "claude-notification-stop",
        "runtime_id": launch_id,
        "provider": "claude",
        "kind": "stop_observed",
        "confidence": "observed"
    }))
    .expect("stop event");
    crate::activity::ingest_event(&state.context, &beta.id, stop).expect("ingest stop");
    let activity_path = session_dir(&state.context, &beta.id).join("activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&activity_path).expect("activity"))
        .expect("activity json");
    activity["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(
        &activity_path,
        serde_json::to_vec_pretty(&activity).expect("serialize activity"),
    )
    .expect("write activity");

    let canary = "CLAUDE-MAILBOX-BODY-MUST-NOT-REACH-PROMPT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "claude-notification-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let candidate = crate::coordination::pending_notifications(&state.context)
        .expect("pending notifications")
        .pop()
        .expect("candidate");

    let transcript = tmp.path().join("claude.jsonl");
    fs::write(&transcript, b"").expect("transcript");
    let attached_tail = ProviderPromptTail::open_path(
        ProviderKind::Claude,
        "provider-beta",
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    let prompt = crate::coordination::notification_prompt(&candidate);
    assert!(matches!(
        start_terminal_coordination_notification(
            &state.context,
            &tmux,
            &candidate,
            &prompt,
            attached_tail,
            AgentKind::Claude,
        ),
        Err(TerminalNotificationStartError::Known("recipient-attached"))
    ));
    assert!(!captured_prompt.exists());

    fs::write(&attached_state, b"0\n").expect("detach session");
    let tail = ProviderPromptTail::open_path(
        ProviderKind::Claude,
        "provider-beta",
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    let tail = start_terminal_coordination_notification(
        &state.context,
        &tmux,
        &candidate,
        &prompt,
        tail,
        AgentKind::Claude,
    )
    .expect("start notification");
    assert_eq!(
        fs::read_to_string(&captured_prompt).expect("captured prompt"),
        prompt
    );
    assert!(!prompt.contains(canary));
    let commands = fs::read_to_string(&command_log).expect("command log");
    assert!(commands.contains("display-message -p -t hs-claude-beta:0.0"));
    assert!(commands.contains("paste-buffer"));
    assert!(!commands.contains("paste-buffer -p"));
    assert!(commands.contains("send-keys -t hs-claude-beta:0.0 Enter"));

    let observed = format!(
        "{}\n",
        json!({
            "type": "user",
            "sessionId": "provider-beta",
            "isSidechain": false,
            "isMeta": false,
            "promptSource": "typed",
            "message": { "role": "user", "content": prompt }
        })
    );
    fs::write(&transcript, observed).expect("append observed prompt");
    assert!(observe_terminal_coordination_prompt(tail, &prompt));
    crate::coordination::mark_notification_submitted(&state.context, &candidate)
        .expect("mark submitted");

    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notification = registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification");
    assert!(!notification.to_string().contains(canary));
    assert_eq!(notification["state"], "prompt_submitted");
}

#[tokio::test]
async fn known_claude_mailbox_dispatch_wakes_once_despite_unrelated_history() {
    let lock = GlobalStateLock::new();
    let _entries = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_RESUME_SCAN_MAX_ENTRIES", "60");
    let _time = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_RESUME_SCAN_SLICE_MS", "60000");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "claude", "hs-claude-beta");
    let provider_root = tmp.path().join("profile-config");
    let projects = provider_root.join("projects");
    for project in 0..55 {
        let dir = projects.join(format!("project-{project:02}"));
        fs::create_dir_all(&dir).expect("project directory");
        for unrelated in 0..8 {
            fs::write(
                dir.join(format!("unrelated-{unrelated}.jsonl")),
                "{}\n".repeat(32),
            )
            .expect("unrelated history");
        }
    }
    let transcript = projects.join("project-27/provider-beta.jsonl");
    let metadata = format!(
        "{}\n",
        json!({"type":"user", "sessionId":"provider-beta", "cwd":"/repo"})
    );
    fs::write(&transcript, format!("{}{}", "{}\n".repeat(6), metadata)).expect("provider metadata");
    let record_path = tmp.path().join("sessions/beta/session.json");
    let mut persisted: Value =
        serde_json::from_slice(&fs::read(&record_path).expect("record")).expect("record json");
    persisted["runtime"]["agent_profile_provider_config_dir"] = json!(provider_root);
    persisted["provider_resume"] = json!({"provider":"claude", "session_id":"provider-beta", "capture_method":"test",
            "captured_at":"2000-01-01T00:00:00Z", "resume_args":[]});
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&persisted).expect("record bytes"),
    )
    .expect("persist provider root");
    let observed_path = tmp.path().join("observed-prompt.jsonl");
    let command_log = tmp.path().join("tmux-commands.log");
    let captured_prompt = tmp.path().join("captured-prompt");
    let attached_state = tmp.path().join("attached-state");
    fs::write(&attached_state, b"0\n").expect("attached state");
    let tmux = tmp.path().join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$1" in
  has-session) exit 0 ;;
  display-message)
read value < {}
printf '%s\n' "$value"
exit 0
;;
  load-buffer)
for value in "$@"; do last="$value"; done
cp "$last" {}
exit 0
;;
  send-keys)
    cat {} >> {}
    exit 0
    ;;
  paste-buffer|delete-buffer) exit 0 ;;
  *) exit 1 ;;
esac
"#,
            shell_words::quote(&command_log.to_string_lossy()),
            shell_words::quote(&attached_state.to_string_lossy()),
            shell_words::quote(&captured_prompt.to_string_lossy()),
            shell_words::quote(&observed_path.to_string_lossy()),
            shell_words::quote(&transcript.to_string_lossy())
        ),
    )
    .expect("tmux script");
    let mut permissions = fs::metadata(&tmux).expect("tmux metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&tmux, permissions).expect("tmux permissions");

    let state = state(tmp.path(), Some(TOKEN), tmux.clone());
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let launch_id = beta
        .runtime
        .as_ref()
        .expect("beta runtime")
        .launch_id
        .clone();
    let stop = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "claude-notification-stop",
        "runtime_id": launch_id,
        "provider": "claude",
        "provider_session_id": beta.provider_resume.as_ref().expect("known provider conversation").session_id,
        "kind": "stop_observed",
        "confidence": "observed"
    }))
    .expect("stop event");
    crate::activity::ingest_event(&state.context, &beta.id, stop).expect("ingest stop");
    let activity_path = session_dir(&state.context, &beta.id).join("activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&activity_path).expect("activity"))
        .expect("activity json");
    activity["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(
        &activity_path,
        serde_json::to_vec_pretty(&activity).expect("serialize activity"),
    )
    .expect("write activity");

    let canary = "CLAUDE-MAILBOX-BODY-MUST-NOT-REACH-PROMPT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "claude-notification-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let candidate = crate::coordination::pending_notifications(&state.context)
        .expect("pending notifications")
        .pop()
        .expect("candidate");

    let prompt = crate::coordination::notification_prompt(&candidate);
    let observed = format!(
        "{}\n",
        json!({
            "type":"user", "sessionId":"provider-beta", "isSidechain":false,
            "isMeta":false, "promptSource":"typed",
            "message":{"role":"user", "content":prompt}
        })
    );
    fs::write(&observed_path, observed).expect("provider's accepted wake record");
    dispatch_coordination_notification(state.clone(), candidate.clone()).await;
    assert_eq!(
        notification_fixture(tmp.path())["state"],
        "prompt_submitted",
        "a readable known transcript must reach submission instead of provider-observation-unavailable"
    );
    assert_eq!(
        fs::read_to_string(&captured_prompt).expect("wake prompt"),
        prompt
    );
    assert!(!prompt.contains(canary));
    assert!(
        crate::coordination::pending_notifications(&state.context)
            .expect("pending")
            .is_empty()
    );
    // A second scheduler tick and a stale candidate must both preserve exactly one wake.
    drain_coordination_notifications(state.clone()).await;
    dispatch_coordination_notification(state.clone(), candidate).await;
    let commands = fs::read_to_string(&command_log).expect("command log");
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.starts_with("paste-buffer "))
            .count(),
        1
    );
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.starts_with("send-keys "))
            .count(),
        1
    );

    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notification = registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification");
    assert!(!notification.to_string().contains(canary));
    assert_eq!(notification["state"], "prompt_submitted");
}
