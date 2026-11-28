use super::*;
use crate::codex_app_server::tests::resume_binding::{
    answer_clear_idle_probe, prepare_clear_runtime,
};
use nils_test_support::{EnvGuard, GlobalStateLock, short_socket_path};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

const CLAUDE_RULE: &str = "────────────────────────────────";
const DRAFT: &str = "synthetic human draft";

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Draft,
    VisibleDialog,
    ReportedDialog,
    Unread,
    UnknownMailbox,
    Clean,
}

fn pane(agent: AgentKind, scenario: Scenario) -> String {
    if matches!(scenario, Scenario::VisibleDialog) {
        return match agent {
            AgentKind::Codex => "› 1. Continue\n  2. Cancel\n  enter to confirm\n".into(),
            AgentKind::Claude => {
                "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel\n".into()
            }
            _ => unreachable!(),
        };
    }
    let text = if matches!(scenario, Scenario::Draft) {
        DRAFT
    } else {
        ""
    };
    match agent {
        AgentKind::Codex => format!("› {text}\n\n  synthetic footer\n"),
        AgentKind::Claude => format!("{CLAUDE_RULE}\n❯ {text}\n{CLAUDE_RULE}\n"),
        _ => unreachable!(),
    }
}

fn unread_mail(context: &CliContext, record: &SessionRecord) -> std::path::PathBuf {
    let registry = context.state_dir.join("coordination/registry.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    write_private_file(
        &registry,
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "messages": [{
                "schema_version": "agent-session.message.v1",
                "message_id": "synthetic-message",
                "sender_session_id": "synthetic-sender",
                "sender_incarnation": "synthetic-incarnation",
                "recipient_session_id": record.id,
                "recipient_incarnation": crate::coordination::incarnation(record).unwrap(),
                "state": "unread",
                "revision": 1,
                "reply_depth": 0,
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_epoch": 1767225600,
                "expires_at": "2099-01-01T00:00:00Z",
                "expires_at_epoch": 4070908800_i64,
                "body": "synthetic",
                "body_bytes": 9
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    registry
}

fn install_fixture_tmux(
    tmp: &tempfile::TempDir,
    context: &CliContext,
    record: &SessionRecord,
    tmux: &std::path::Path,
) {
    for (name, value) in [
        ("AGENT_SESSION_ID", record.id.clone()),
        (
            "AGENT_SESSION_STATE_DIR",
            crate::display_path(&context.state_dir),
        ),
        (
            "AGENT_SESSION_RUNTIME_ID",
            record.runtime.as_ref().unwrap().launch_id.clone(),
        ),
    ] {
        fs::write(tmp.path().join(name), format!("{name}={value}\n")).unwrap();
    }
    fs::write(
        tmux,
        r#"#!/bin/sh
root=${0%/*}
case "$1" in
  has-session) printf '.\n' >> "$root/status-probes" ;;
  capture-pane) cat "$root/pane" ;;
  show-environment) cat "$root/$4" ;;
  load-buffer) cat "$4" > "$root/buffer" ;;
  paste-buffer) cat "$root/buffer" >> "$root/pane" ;;
  send-keys)
    printf '%s\n' "$4" >> "$root/keys"
    cat "$root/buffer" >> "$root/submissions"
    printf '\n' >> "$root/submissions"
    ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(tmux, fs::Permissions::from_mode(0o700)).unwrap();
}

async fn check_clear(
    agent: AgentKind,
    scenario: Scenario,
    allow_unread_mail: bool,
    refusal: Option<&str>,
) {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, mut record, tmux) = tests::fixture(&tmp, agent);
    let (_socket_dir, socket) = short_socket_path("clear-probe.sock");
    let (_proxy, server) = if agent == AgentKind::Codex {
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let proxy = prepare_clear_runtime(&context, &mut record, &socket);
        let server = tokio::spawn(answer_clear_idle_probe(listener, "old-conversation".into()));
        (Some(proxy), Some(server))
    } else {
        (None, None)
    };

    let pane_path = tmp.path().join("pane");
    let pane_before = pane(agent, scenario);
    fs::write(&pane_path, &pane_before).unwrap();
    install_fixture_tmux(&tmp, &context, &record, &tmux);

    let dir = session_dir(&context, &record.id);
    let activity_path = dir.join("activity.json");
    if matches!(scenario, Scenario::ReportedDialog) {
        let mut activity: serde_json::Value =
            serde_json::from_slice(&fs::read(&activity_path).unwrap()).unwrap();
        activity["state"]["phase"] = json!("needs_input");
        write_private_file(&activity_path, &serde_json::to_vec(&activity).unwrap()).unwrap();
    }
    let mut protected = vec![
        dir.join("session.json"),
        dir.join("resume.json"),
        activity_path,
    ];
    let registry = (allow_unread_mail || matches!(scenario, Scenario::Unread))
        .then(|| unread_mail(&context, &record));
    if let Some(path) = &registry {
        if matches!(scenario, Scenario::UnknownMailbox) {
            write_private_file(path, b"invalid registry").unwrap();
        }
        protected.push(path.clone());
    }
    let registry_before = registry.as_ref().map(|path| fs::read(path).unwrap());
    let before: Vec<_> = protected
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let activity_before = activity::controller_state_for_record(&context, &record).unwrap();

    let observer = refusal.is_none().then(|| {
        let context = context.clone();
        let record = record.clone();
        let keys = tmp.path().join("keys");
        thread::spawn(move || {
            for _ in 0..150 {
                if fs::read_to_string(&keys).is_ok_and(|keys| keys == "Enter\n") {
                    observe_native(&context, &record, "new-conversation").unwrap();
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("native slash command was not submitted");
        })
    });
    let command_context = context.clone();
    let id = record.id.clone();
    let result = tokio::task::spawn_blocking(move || {
        operation(
            &command_context,
            ConversationArgs {
                id,
                expect_idle: true,
                timeout: 1,
                tmux_bin: Some(tmux),
                format: crate::OutputFormat::Json,
            },
            Operation::Clear(ClearOptions {
                allow_unread_mail,
                ..Default::default()
            }),
        )
    })
    .await
    .unwrap();
    if let Some(server) = server {
        if !server.is_finished() {
            server.abort();
        }
        let _ = server.await;
    }
    if let Some(observer) = observer {
        observer.join().unwrap();
    }

    if let Some(code) = refusal {
        assert_eq!(
            result.as_ref().err().map(CliError::code),
            Some(code),
            "{agent:?} {scenario:?}: {result:?}"
        );
        assert_eq!(fs::read_to_string(&pane_path).unwrap(), pane_before);
        assert!(!tmp.path().join("buffer").exists());
        assert!(!tmp.path().join("keys").exists());
        assert!(live(&context, &record).unwrap().is_none());
        for (path, bytes) in protected.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes, "{path:?}");
        }
        assert_eq!(
            activity::controller_state_for_record(&context, &record).unwrap(),
            activity_before,
        );
        if !matches!(scenario, Scenario::ReportedDialog) {
            activity::refuse_if_blocked(&context, &record, "resolve the provider prompt").unwrap();
        }
    } else {
        let result = result.unwrap();
        assert_eq!(result.new_provider_session_id, "new-conversation");
        if let Some((registry, before)) = registry.zip(registry_before) {
            assert_eq!(fs::read(registry).unwrap(), before);
        }
        let slash_command = if agent == AgentKind::Codex {
            "/new"
        } else {
            "/clear"
        };
        assert_eq!(
            fs::read_to_string(tmp.path().join("buffer")).unwrap(),
            slash_command
        );
    }
}

#[tokio::test]
async fn clear_admission_refuses_human_drafts() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(
            agent,
            Scenario::Draft,
            false,
            Some("send-composer-not-empty"),
        )
        .await;
    }
}

#[tokio::test]
async fn clear_admission_refuses_visible_dialogs_before_reporting() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(
            agent,
            Scenario::VisibleDialog,
            false,
            Some("send-composer-unobservable"),
        )
        .await;
    }
}

#[tokio::test]
async fn clear_admission_refuses_reported_dialogs() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(
            agent,
            Scenario::ReportedDialog,
            false,
            Some("agent-blocked"),
        )
        .await;
    }
}

#[tokio::test]
async fn clear_admission_refuses_unread_mail() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(
            agent,
            Scenario::Unread,
            false,
            Some("session-mailbox-not-empty"),
        )
        .await;
    }
}

#[tokio::test]
async fn clear_admission_accepts_clean_session() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(agent, Scenario::Clean, false, None).await;
    }
}

#[tokio::test]
async fn clear_admission_override_preserves_unread_mail() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_clear(agent, Scenario::Unread, true, None).await;
    }
}

#[tokio::test]
async fn clear_admission_override_still_refuses_unsafe_sessions() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        for (scenario, code) in [
            (Scenario::Draft, "send-composer-not-empty"),
            (Scenario::VisibleDialog, "send-composer-unobservable"),
            (Scenario::ReportedDialog, "agent-blocked"),
            (Scenario::UnknownMailbox, "session-mailbox-unknown"),
        ] {
            check_clear(agent, scenario, true, Some(code)).await;
        }
    }
}

#[derive(Clone, Copy)]
enum InitialPrompt {
    Draft,
    Confirmed,
    Unconfirmed,
    NeverIdle,
}

async fn check_initial_prompt(agent: AgentKind, scenario: InitialPrompt) {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, mut record, tmux) = tests::fixture(&tmp, agent);
    let (_socket_dir, socket) = short_socket_path("initial-prompt.sock");
    let (_proxy, server) = if agent == AgentKind::Codex {
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let proxy = prepare_clear_runtime(&context, &mut record, &socket);
        let server = tokio::spawn(answer_clear_idle_probe(listener, "old-conversation".into()));
        (Some(proxy), Some(server))
    } else {
        (None, None)
    };

    const PROMPT: &str = "Synthetic first prompt\nSecond line";
    let prompt = tmp.path().join("prompt.txt");
    fs::write(&prompt, PROMPT).unwrap();
    install_fixture_tmux(&tmp, &context, &record, &tmux);
    let pane_before = pane(
        agent,
        if matches!(scenario, InitialPrompt::Draft) {
            Scenario::Draft
        } else {
            Scenario::Clean
        },
    );
    let pane_file = tmp.path().join("pane");
    fs::write(&pane_file, &pane_before).unwrap();
    unread_mail(&context, &record);
    let activity_path = session_dir(&context, &record.id).join("activity.json");
    let waiting = fs::read(&activity_path).unwrap();
    if matches!(
        scenario,
        InitialPrompt::Confirmed | InitialPrompt::NeverIdle
    ) {
        let mut working: serde_json::Value = serde_json::from_slice(&waiting).unwrap();
        working["state"]["phase"] = json!("working");
        write_private_file(&activity_path, &serde_json::to_vec(&working).unwrap()).unwrap();
    }
    let submissions = tmp.path().join("submissions");
    let slash = if agent == AgentKind::Claude {
        "/clear"
    } else {
        "/new"
    };
    let observer = matches!(scenario, InitialPrompt::Confirmed).then(|| {
        let context = context.clone();
        let record = record.clone();
        let submissions = submissions.clone();
        let pane_file = pane_file.clone();
        let pane_before = pane_before.clone();
        let probes = tmp.path().join("status-probes");
        thread::spawn(move || {
            for _ in 0..200 {
                if fs::read_to_string(&probes).is_ok_and(|calls| calls.lines().count() >= 2) {
                    assert!(
                        !submissions.exists(),
                        "working session was not yet safe to clear"
                    );
                    write_private_file(&activity_path, &waiting).unwrap();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            for _ in 0..250 {
                if fs::read_to_string(&submissions).is_ok_and(|value| value == format!("{slash}\n"))
                {
                    let incarnation = coordination::incarnation(&record).unwrap();
                    let mut quiescence = coordination::lock_session_quiescence_observational(
                        &context,
                        &record.id,
                        &incarnation,
                    )
                    .unwrap();
                    // This fake receiver has no real broker. Hold all other
                    // wake predicates constant to test the actual input fence.
                    quiescence.broker_authoritative = true;
                    assert_eq!(quiescence.notification_block(false), Some("clear-fenced"));
                    drop(quiescence);
                    assert_eq!(
                        ensure_input_available(&context, &record)
                            .unwrap_err()
                            .code(),
                        "conversation-clear-in-progress",
                    );
                    fs::write(&pane_file, &pane_before).unwrap();
                    observe_native(&context, &record, "new-conversation").unwrap();
                    for _ in 0..300 {
                        let mut quiescence = coordination::lock_session_quiescence_observational(
                            &context,
                            &record.id,
                            &incarnation,
                        )
                        .unwrap();
                        quiescence.broker_authoritative = true;
                        if quiescence.notification_block(false).is_none() {
                            assert_eq!(
                                fs::read_to_string(&submissions).unwrap(),
                                format!("{slash}\n{PROMPT}\n")
                            );
                            return;
                        }
                        drop(quiescence);
                        thread::sleep(Duration::from_millis(10));
                    }
                    panic!("mailbox wake remained fenced after clear");
                }
                thread::sleep(Duration::from_millis(10));
            }
        })
    });
    let args = [
        "agent-session",
        "clear",
        &record.id,
        "--state-dir",
        context.state_dir.to_str().unwrap(),
        "--tmux-bin",
        tmux.to_str().unwrap(),
        "--wait-idle",
        "2",
        "--timeout",
        "1",
        "--initial-prompt-file",
        prompt.to_str().unwrap(),
        "--format",
        "json",
    ]
    .map(str::to_owned);
    let result = tokio::task::spawn_blocking(move || crate::run_with_args(args))
        .await
        .unwrap();
    if let Some(server) = server {
        server.abort();
        let _ = server.await;
    }
    if let Some(observer) = observer {
        observer.join().unwrap();
    }
    if matches!(scenario, InitialPrompt::Draft | InitialPrompt::NeverIdle) {
        assert_eq!(
            result, 1,
            "clear must refuse at runtime, not at CLI parsing"
        );
        assert_eq!(fs::read_to_string(&pane_file).unwrap(), pane_before);
        assert!(!submissions.exists());
    } else {
        let expected = if matches!(scenario, InitialPrompt::Confirmed) {
            format!("{slash}\n{PROMPT}\n")
        } else {
            format!("{slash}\n")
        };
        assert_eq!(
            fs::read_to_string(&submissions).unwrap_or_default(),
            expected,
            "only the native clear and, after confirmation, exactly one first prompt may be submitted"
        );
        assert_eq!(
            result,
            i32::from(matches!(scenario, InitialPrompt::Unconfirmed))
        );
    }

    let dir = session_dir(&context, &record.id).join("clear-receipts");
    let paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    let raw = fs::read_to_string(&paths[0]).unwrap();
    let receipt: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(!raw.contains("Synthetic first prompt"));
    assert_eq!(
        receipt["session_incarnation"],
        coordination::incarnation(&record).unwrap()
    );
    assert_eq!(
        receipt["prompt_sha256"],
        coordination::digest_bytes(PROMPT.as_bytes())
    );
    match scenario {
        InitialPrompt::Draft => assert_eq!(receipt["error_code"], "send-composer-not-empty"),
        InitialPrompt::Confirmed => {
            assert_eq!(receipt["new_provider_session_id"], "new-conversation");
            assert_eq!(receipt["prompt_sent"], true);
        }
        InitialPrompt::NeverIdle => {
            assert_eq!(receipt["error_code"], "conversation-idle-timeout");
            assert_eq!(receipt["outcome"], "timed-out");
            assert!(receipt["clear_submitted_at"].is_null());
            assert_eq!(receipt["prompt_sent"], false);
        }
        InitialPrompt::Unconfirmed => {
            assert_eq!(receipt["error_code"], "conversation-clear-unconfirmed");
            assert_eq!(receipt["outcome"], "timed-out");
            assert!(receipt["new_provider_session_id"].is_null());
            assert_eq!(receipt["prompt_sent"], false);
        }
    }
    assert_eq!(
        coordination::unread::check(&context, &record.id, false, "fixture")
            .unwrap_err()
            .code(),
        "session-mailbox-not-empty"
    );
}

#[tokio::test]
async fn clear_initial_prompt_refuses_and_preserves_a_draft() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_initial_prompt(agent, InitialPrompt::Draft).await;
    }
}

#[tokio::test]
async fn clear_initial_prompt_is_submitted_exactly_once_after_confirmation() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_initial_prompt(agent, InitialPrompt::Confirmed).await;
    }
}

#[tokio::test]
async fn clear_initial_prompt_is_not_submitted_on_confirmation_timeout() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        check_initial_prompt(agent, InitialPrompt::Unconfirmed).await;
    }
}

#[tokio::test]
async fn clear_wait_idle_timeout_does_not_submit_clear() {
    check_initial_prompt(AgentKind::Codex, InitialPrompt::NeverIdle).await;
}
