use super::*;
use clap::Parser;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

#[test]
fn start_notice_uses_shared_encoder_for_each_prompt_source() {
    let lock = GlobalStateLock::new();
    let _enabled = EnvGuard::set(&lock, WRITERS_ENV, "1");
    let temp = tempfile::tempdir().unwrap();
    let body = "  Initial synthetic prompt\r\n\nUnicode 本文\n";
    let file = temp.path().join("prompt");
    std::fs::write(&file, body).unwrap();
    for source in [
        vec!["--prompt", body],
        vec!["--prompt-file", file.to_str().unwrap()],
    ] {
        let mut flags = vec![
            "agent-session",
            "start",
            "--agent",
            "claude",
            "--notice-kind",
            "dispatch.bootstrap",
            "--notice-source",
            "scheduler",
        ];
        flags.extend(source);
        let cli::Command::Start(mut args) = cli::Cli::try_parse_from(flags).unwrap().command else {
            panic!()
        };
        prepare_start(&mut args).unwrap();
        assert_eq!(
            args.prompt.as_deref(),
            Some(
                NoticeEnvelope::new("dispatch.bootstrap", "scheduler", None)
                    .unwrap()
                    .encode(body)
                    .unwrap()
                    .as_str()
            )
        );
        assert!(args.prompt_file.is_none());
        assert!(!args.prompt_stdin);
    }
}

#[test]
fn ordinary_start_does_not_apply_notice_gate_or_change_prompt() {
    let lock = GlobalStateLock::new();
    let _disabled = EnvGuard::remove(&lock, WRITERS_ENV);
    let cli::Command::Start(mut args) = cli::Cli::try_parse_from([
        "agent-session",
        "start",
        "--agent",
        "codex",
        "--prompt",
        "human text\n",
    ])
    .unwrap()
    .command
    else {
        panic!()
    };
    prepare_start(&mut args).unwrap();
    assert_eq!(args.prompt.as_deref(), Some("human text\n"));
}

#[test]
fn history_notice_bytes_survive_existing_provider_admission() {
    use crate::provider_prompt::parse_history_user_prompt;
    use serde_json::json;
    let body = "Mailbox: agent-session message inbox";
    let marked = NoticeEnvelope::new("mailbox", "agent-session", None)
        .unwrap()
        .encode(body)
        .unwrap();
    let claude = |text: &str| {
        json!({"type":"user", "sessionId":"synthetic", "promptSource":"typed",
        "message":{"role":"user", "content":text}})
    };
    let codex = json!({"type":"response_item", "payload":{"type":"message", "role":"user",
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]},
        "content":[{"type":"input_text", "text":marked}]}});
    let codex_event =
        json!({"type":"event_msg", "payload":{"type":"user_message", "message":marked}});
    for (provider, value) in [
        ("claude", claude(&marked)),
        ("codex", codex.clone()),
        ("codex", codex_event),
    ] {
        let prompt = parse_history_user_prompt(provider, "synthetic", &value.to_string()).unwrap();
        assert_eq!(prompt.text.as_bytes(), marked.as_bytes());
    }
    assert!(parse_history_user_prompt("claude", "synthetic", &claude(body).to_string()).is_none());
    for field in [
        "isMeta",
        "isCompactSummary",
        "isVisibleInTranscriptOnly",
        "isSidechain",
    ] {
        let mut value = claude(&marked);
        value[field] = json!(true);
        assert!(parse_history_user_prompt("claude", "synthetic", &value.to_string()).is_none());
    }
    let mut system = claude(&marked);
    system["promptSource"] = json!("system");
    assert!(parse_history_user_prompt("claude", "synthetic", &system.to_string()).is_none());
    let mut internal = codex;
    internal["payload"]["internal_chat_message_metadata_passthrough"]["content_item_kinds"] =
        json!(["goal.internal_context"]);
    assert!(parse_history_user_prompt("codex", "synthetic", &internal.to_string()).is_none());
}
