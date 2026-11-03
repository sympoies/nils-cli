use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io::Write;

#[test]
fn runtime_commands_are_live_display_rows_but_never_human_prompt_events() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.jsonl");
        let initial = match provider {
            ProviderKind::Claude => {
                json!({"type":"user","sessionId":"synthetic","promptSource":"typed","message":{"role":"user","content":"human prompt"}})
            }
            ProviderKind::Codex => {
                json!({"type":"event_msg","payload":{"type":"user_message","message":"human prompt"}})
            }
        };
        fs::write(&path, format!("{initial}\n")).unwrap();
        let mut warm = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        for token in [
            "/compact", "/exit", "/quit", "/effort", "/model", "/usage", "/login", "/clear", "/new",
        ] {
            let text = format!("{token}\t keep 🌱 focus  ");
            let row = match provider {
                ProviderKind::Claude => {
                    json!({"type":"user","sessionId":"synthetic","message":{"role":"user","content":format!("<command-name>{token}</command-name>\n<command-message>{}</command-message>\n<command-args>\t keep 🌱 focus  </command-args>",token.trim_start_matches('/'))}})
                }
                ProviderKind::Codex => {
                    json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
                }
            };
            let mut tail =
                ProviderPromptTail::open_path(provider, "synthetic", path.clone(), Duration::ZERO)
                    .unwrap();
            writeln!(
                fs::OpenOptions::new().append(true).open(&path).unwrap(),
                "{row}"
            )
            .unwrap();
            assert_eq!(tail.poll_submitted_prompts().unwrap(), Vec::<String>::new());
            let ProviderLastPromptRefresh::Current(Some(prompt)) = warm.refresh() else {
                panic!("current")
            };
            assert_eq!(prompt.text, "human prompt");
            let display = serde_json::to_value(warm.last_message().unwrap()).unwrap();
            assert_eq!(display["row_class"], "runtime_command");
            let expected = if provider == ProviderKind::Claude {
                format!("{token} \t keep 🌱 focus  ")
            } else {
                text
            };
            assert_eq!(display["text"], expected);
            let mut cold = ProviderLastPromptTracker::open_path(
                provider,
                "synthetic",
                path.clone(),
                Duration::ZERO,
            )
            .unwrap();
            assert_eq!(
                cold.refresh(),
                ProviderLastPromptRefresh::Current(Some(prompt))
            );
            assert_eq!(cold.last_message(), warm.last_message());
        }
    }
}

#[test]
fn compaction_card_preview_matches_history_summary_and_never_replaces_the_human_prompt() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.jsonl");
        let initial = match provider {
            ProviderKind::Claude => {
                json!({"type":"user","sessionId":"synthetic","promptSource":"typed","message":{"role":"user","content":"human prompt"}})
            }
            ProviderKind::Codex => {
                json!({"type":"event_msg","payload":{"type":"user_message","message":"human prompt"}})
            }
        };
        fs::write(&path, format!("{initial}\n")).unwrap();
        let mut warm = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        for length in [65536, 65537] {
            let summary = format!(" {}", "🌱".repeat(length - 1));
            let rows = match provider {
                ProviderKind::Claude => vec![
                    json!({"type":"system","subtype":"compact_boundary"}),
                    json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":summary}}),
                ],
                ProviderKind::Codex => {
                    vec![json!({"type":"compacted","payload":{"message":summary}})]
                }
            };
            for row in rows {
                writeln!(
                    fs::OpenOptions::new().append(true).open(&path).unwrap(),
                    "{row}"
                )
                .unwrap();
            }
            let ProviderLastPromptRefresh::Current(Some(prompt)) = warm.refresh() else {
                panic!("current")
            };
            assert_eq!(prompt.text, "human prompt");
            let display = serde_json::to_value(warm.last_message().unwrap()).unwrap();
            assert_eq!(display["row_class"], "compaction");
            assert_eq!(
                display["text"],
                summary.chars().take(65536).collect::<String>()
            );
            assert_eq!(display["compaction"]["summary_truncated"], length > 65536);
            let mut cold = ProviderLastPromptTracker::open_path(
                provider,
                "synthetic",
                path.clone(),
                Duration::ZERO,
            )
            .unwrap();
            assert_eq!(
                cold.refresh(),
                ProviderLastPromptRefresh::Current(Some(prompt))
            );
            assert_eq!(cold.last_message(), warm.last_message());
        }
    }
}

#[test]
fn cold_and_warm_card_previews_wait_for_the_same_completed_compaction_record() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.jsonl");
        let initial = match provider {
            ProviderKind::Claude => {
                json!({"type":"user","sessionId":"synthetic","message":{"role":"user","content":"human"}})
            }
            ProviderKind::Codex => {
                json!({"type":"event_msg","payload":{"type":"user_message","message":"human"}})
            }
        };
        fs::write(&path, format!("{initial}\n")).unwrap();
        let mut warm = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        let record = if provider == ProviderKind::Claude {
            writeln!(
                fs::OpenOptions::new().append(true).open(&path).unwrap(),
                "{}",
                json!({"type":"system","subtype":"compact_boundary"})
            )
            .unwrap();
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary\n"}})
        } else {
            json!({"type":"compacted","payload":{"message":"summary\n"}})
        };
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(record.to_string().as_bytes())
            .unwrap();
        let mut cold = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        warm.refresh();
        cold.refresh();
        assert_eq!(cold.last_message(), warm.last_message());
        assert_eq!(warm.last_message().unwrap().text, "human");
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        warm.refresh();
        cold.refresh();
        assert_eq!(cold.last_message(), warm.last_message());
        assert_eq!(warm.last_message().unwrap().text, "summary\n");
    }
}

#[test]
fn foreign_claude_records_cannot_replace_the_current_compaction_card() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("synthetic.jsonl");
    let opening =
        json!({"type":"user","sessionId":"synthetic","message":{"role":"user","content":"human"}});
    fs::write(&path, format!("{opening}\n")).unwrap();
    let mut warm = ProviderLastPromptTracker::open_path(
        ProviderKind::Claude,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    for key in ["sessionId", "session_id"] {
        let boundary =
            json!({"type":"system","subtype":"compact_boundary","sessionId":"synthetic"});
        let foreign = json!({"type":"user","isCompactSummary":true,key:"other","message":{"role":"user","content":"foreign"}});
        let summary = json!({"type":"user","isCompactSummary":true,"sessionId":"synthetic","message":{"role":"user","content":"current"}});
        for row in [
            boundary,
            foreign,
            summary,
            json!({"type":"system","subtype":"compact_boundary",key:"other"}),
        ] {
            writeln!(
                fs::OpenOptions::new().append(true).open(&path).unwrap(),
                "{row}"
            )
            .unwrap();
        }
        warm.refresh();
        let mut cold = ProviderLastPromptTracker::open_path(
            ProviderKind::Claude,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        cold.refresh();
        assert_eq!(cold.last_message(), warm.last_message());
        assert_eq!(warm.last_message().unwrap().text, "current");
    }
}

#[test]
fn codex_notice_submission_is_observable_without_a_human_prompt_event() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("synthetic.jsonl");
    let opening =
        json!({"type":"event_msg","payload":{"type":"user_message","message":"human prompt"}});
    fs::write(&path, format!("{opening}\n")).unwrap();
    let mut submissions = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    let mut human_events = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    let mut card = ProviderLastPromptTracker::open_path(
        ProviderKind::Codex,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    let text = "Mailbox: agent-session message inbox";
    let rows = [
        json!({"type":"event_msg","payload":{"type":"user_message","message":text}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}}),
    ];
    for row in rows {
        writeln!(
            fs::OpenOptions::new().append(true).open(&path).unwrap(),
            "{row}"
        )
        .unwrap();
    }
    assert_eq!(submissions.poll_submitted_prompts().unwrap(), vec![text]);
    assert!(submissions.poll_submitted_prompts().unwrap().is_empty());
    assert!(human_events.poll().unwrap().is_empty());
    let ProviderLastPromptRefresh::Current(Some(prompt)) = card.refresh() else {
        panic!("current")
    };
    assert_eq!(prompt.text, "human prompt");
    let display = serde_json::to_value(card.last_message().unwrap()).unwrap();
    assert_eq!(display["row_class"], "notice");
    assert_eq!(display["text"], text);
    let internal = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["goal.internal_context"]}}});
    writeln!(
        fs::OpenOptions::new().append(true).open(&path).unwrap(),
        "{internal}"
    )
    .unwrap();
    assert!(submissions.poll_submitted_prompts().unwrap().is_empty());
    assert!(human_events.poll().unwrap().is_empty());
}
