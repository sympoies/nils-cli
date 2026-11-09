//! Modeled producer-ledger fixtures; never evidence of native execution.
use super::*;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn codex_attested_notice_keeps_response_row_identity_exact_bytes_and_human_control() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let provider = "codex-notice-thread";
    let history = stable_history_id("codex", None, provider);
    let text = "<!-- agent-notice:v1 kind=runtime.resume source=agent-session -->\n\nResume  \n";
    let baseline = format!(
        "{}\n",
        serde_json::json!({"type":"session_meta","timestamp":"2026-09-01T00:00:00Z",
        "payload":{"id":provider,"cwd":"/work/example","source":"cli","timestamp":"2026-09-01T00:00:00Z"}})
    );
    let response = serde_json::json!({"type":"response_item","payload":{"type":"message",
        "role":"user","content":[{"type":"input_text","text":text}],
        "internal_chat_message_metadata_passthrough":{"turn_id":"accepted-turn","content_item_kinds":["user.text"]}}}).to_string();
    let event = serde_json::json!({"type":"event_msg","payload":{"type":"user_message",
        "client_id":"agent-session-fixture","message":text,"images":[],"local_images":[]}})
    .to_string();
    let human = serde_json::json!({"type":"response_item","payload":{"type":"message",
        "role":"user","content":[{"type":"input_text","text":text}]}});
    let transcript = root.join("provider/session.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(
        &transcript,
        format!("{baseline}{response}\n{event}\n{human}\n"),
    )
    .unwrap();
    let op = "33333333-3333-4333-8333-333333333333";
    let claim = serde_json::json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"claim","op":op,"history_id":history,
        "binding":{"launch":{"session_id":"modeled","incarnation":"runtime-one","generation":1,"profile":null},
            "provider":"codex","provider_session_id":provider},
        "codex":{"identity_sha256":"a".repeat(64),"client_message_id":"agent-session-fixture"},
        "envelope_bytes":text.len(),"envelope_sha256":digest(text.as_bytes()),"original_budget_ms":15000,
        "baseline":{"device":0,"inode":0,"eof":baseline.len(),"continuity_sha256":digest(baseline.as_bytes())}});
    let claim_bytes = claim.to_string();
    let terminal = serde_json::json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"terminal","op":op,"claim_sha256":digest(claim_bytes.as_bytes()),"state":"submitted",
        "native_uuid":digest(response.as_bytes()),"row_offset":baseline.len(),
        "row_payload_sha256":digest(response.as_bytes()),"envelope_sha256":digest(text.as_bytes()),
        "confirmation_elapsed_ms":10,
        "codex":{"event_offset":baseline.len()+response.len()+1,"event_payload_sha256":digest(event.as_bytes()),"turn_id":"accepted-turn"}});
    let state = root.join("state");
    let directory = state.join("history/claude-notices");
    fs::create_dir_all(&directory).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = directory.join("ledger.jsonl");
    fs::write(&ledger, format!("{claim_bytes}\n{terminal}\n")).unwrap();
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: root.join("provider"),
        }],
        archive_root(&state),
        star_root(&state),
    )
    .with_session_state_dir(state);
    for direction in [
        HistoryMessageDirection::Forward,
        HistoryMessageDirection::Latest,
    ] {
        let page = catalog
            .messages_with_parts(&history, None, 10, direction, true)
            .unwrap();
        assert_eq!(page.messages.len(), 2);
        let notice = &page.messages[0];
        assert_eq!(
            notice.role, "notice",
            "the producer-attested physical row must be system text"
        );
        assert_eq!(notice.id, format!("{history}-{}", baseline.len()));
        assert_eq!(notice.text, text);
        assert!(!notice.human_prompt);
        assert!(notice.prompt_request.is_none());
        assert!(notice.images.is_empty());
        assert!(notice.parts.as_ref().unwrap().is_empty());
        assert_eq!(
            page.messages[1].role, "user",
            "an identical human envelope has no machine authority"
        );
    }
}

fn notice_catalog(root: &Path, text: &str) -> (HistoryCatalog, String, Vec<String>) {
    let provider_id = "11111111-1111-4111-8111-111111111111";
    let row_id = "22222222-2222-4222-8222-222222222222";
    let op = "33333333-3333-4333-8333-333333333333";
    let history_id = stable_history_id("claude", None, provider_id);
    let directory = root.join("provider/project");
    fs::create_dir_all(&directory).unwrap();
    let human_text = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nhuman words";
    let human = serde_json::json!({"type":"user", "sessionId":provider_id,
        "uuid":"44444444-4444-4444-8444-444444444444", "cwd":"/work/example",
        "promptSource":"typed", "version":"2.1.296", "message":{"role":"user","content":human_text}});
    let notice = serde_json::json!({"type":"user", "sessionId":provider_id,
        "uuid":row_id, "cwd":"/work/example", "promptSource":"system", "isSidechain":false,
        "version":"2.1.296", "timestamp":"2026-09-01T00:00:00Z",
        "origin":{"kind":"plugin","name":"agent-session-claude-notice","asUser":true},
        "message":{"role":"user","content":[{"type":"text","text":text}]}});
    let assistant = serde_json::json!({"type":"assistant", "sessionId":provider_id,
        "message":{"role":"assistant","content":[{"type":"text","text":"response"}]}});
    let human_row = human.to_string() + "\n";
    let notice_row = notice.to_string();
    let offset = human_row.len() as u64;
    fs::write(
        directory.join(format!("{provider_id}.jsonl")),
        format!("{human_row}{notice_row}\n{assistant}\n"),
    )
    .unwrap();
    let binding = serde_json::json!({"launch":{"session_id":"modeled", "incarnation":"runtime-one",
        "generation":1,"profile":null}, "provider":"claude", "provider_session_id":provider_id,
        "provider_version":"2.1.296", "module_sha256":"a".repeat(64)});
    let claim = serde_json::json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"claim", "op":op, "binding":binding, "history_id":history_id,
        "envelope_bytes":text.len(), "envelope_sha256":digest(text.as_bytes()),
        "original_budget_ms":15000,
        "baseline":{"device":0,"inode":0,"eof":offset,"continuity_sha256":digest(human_row.as_bytes())}});
    let claim_bytes = claim.to_string();
    let terminal = serde_json::json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"terminal", "op":op, "claim_sha256":digest(claim_bytes.as_bytes()),
        "state":"submitted", "native_uuid":row_id, "row_offset":offset,
        "row_payload_sha256":digest(notice_row.as_bytes()), "envelope_sha256":digest(text.as_bytes()),
        "timestamp":"2026-09-01T00:00:00Z", "confirmation_elapsed_ms":10});
    let state = root.join("state");
    let ledger_dir = state.join("history/claude-notices");
    fs::create_dir_all(&ledger_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&ledger_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = ledger_dir.join("ledger.jsonl");
    fs::write(&ledger, format!("{claim_bytes}\n{terminal}\n")).unwrap();
    fs::set_permissions(ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: root.join("provider"),
        }],
        archive_root(&state),
        star_root(&state),
    )
    .with_session_state_dir(state);
    (
        catalog,
        history_id,
        vec![human_text.into(), text.into(), "response".into()],
    )
}

#[test]
fn native_notice_bound_history_keeps_exact_text_and_physical_paging() {
    for body in [
        "Alpha  \nBeta \n".to_string(),
        (0..36).map(|i| format!("row {i}  \n")).collect(),
    ] {
        let text = format!("<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\n{body}");
        for include_parts in [false, true] {
            let temp = tempfile::TempDir::new().unwrap();
            let (catalog, id, expected_texts) = notice_catalog(temp.path(), &text);
            let expected_parts = include_parts.then(|| serde_json::json!([]));
            for reverse in [false, true] {
                let mut cursor = None;
                let mut messages = Vec::new();
                for _ in 0..8 {
                    let direction = if reverse {
                        if cursor.is_none() {
                            HistoryMessageDirection::Latest
                        } else {
                            HistoryMessageDirection::Older
                        }
                    } else {
                        HistoryMessageDirection::Forward
                    };
                    let page = catalog
                        .messages_with_parts(&id, cursor.as_deref(), 1, direction, include_parts)
                        .unwrap();
                    cursor = if reverse {
                        page.older_cursor
                    } else {
                        page.next_cursor
                    };
                    if reverse {
                        messages.splice(0..0, page.messages);
                    } else {
                        messages.extend(page.messages);
                    }
                    if cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(
                    messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
                    ["user", "notice", "assistant"]
                );
                assert_eq!(
                    messages.iter().map(|m| m.text.clone()).collect::<Vec<_>>(),
                    expected_texts
                );
                assert_eq!(messages[1].images.len(), 0);
                assert!(!messages[1].human_prompt);
                let value = serde_json::to_value(&messages[1]).unwrap();
                assert_eq!(value["text"], text);
                assert!(value.get("prompt_request").is_none());
                assert_eq!(value.get("parts"), expected_parts.as_ref());
            }
            let latest = catalog
                .messages_with_parts(&id, None, 2, HistoryMessageDirection::Latest, include_parts)
                .unwrap();
            let notice = latest.messages.iter().find(|m| m.role == "notice").unwrap();
            let value = serde_json::to_value(notice).unwrap();
            assert_eq!(value.get("parts"), expected_parts.as_ref());
        }
    }
}

#[test]
fn native_notice_unproved_or_conflicting_rows_never_become_human_messages() {
    let text = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nsynthetic  \n";
    for mode in [
        "missing",
        "unfinished",
        "unknown",
        "wrong_hash",
        "changed_row",
        "ambiguous",
        "invalid_envelope",
    ] {
        let temp = tempfile::TempDir::new().unwrap();
        let (catalog, id, _) = notice_catalog(temp.path(), text);
        let ledger = temp
            .path()
            .join("state/history/claude-notices/ledger.jsonl");
        let mut rows: Vec<Value> = fs::read_to_string(&ledger)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        match mode {
            "missing" => {
                fs::remove_file(&ledger).unwrap();
            }
            "unfinished" => {
                rows.truncate(1);
            }
            "unknown" => {
                rows[1] = serde_json::json!({"schema_version":"agent-session.claude-notice-ledger.v1", "kind":"terminal",
                    "op":rows[0]["op"],"claim_sha256":digest(rows[0].to_string().as_bytes()),"state":"unknown","reason":"timeout"});
            }
            "wrong_hash" => {
                rows[1]["row_payload_sha256"] = serde_json::json!("b".repeat(64));
            }
            "changed_row" | "invalid_envelope" => {
                let transcript = temp
                    .path()
                    .join("provider/project/11111111-1111-4111-8111-111111111111.jsonl");
                let mut records: Vec<Value> = fs::read_to_string(&transcript)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                let changed = if mode == "changed_row" {
                    "edited human words"
                } else {
                    "<!-- agent-notice:v99 -->\n\ninvalid"
                };
                records[1]["message"]["content"][0]["text"] = serde_json::json!(changed);
                if mode == "changed_row" {
                    records[1]["promptSource"] = serde_json::json!("typed");
                } else {
                    rows[0]["envelope_bytes"] = serde_json::json!(changed.len());
                    rows[0]["envelope_sha256"] = serde_json::json!(digest(changed.as_bytes()));
                    rows[1]["claim_sha256"] =
                        serde_json::json!(digest(rows[0].to_string().as_bytes()));
                    rows[1]["envelope_sha256"] = rows[0]["envelope_sha256"].clone();
                    rows[1]["row_payload_sha256"] =
                        serde_json::json!(digest(records[1].to_string().as_bytes()));
                }
                fs::write(
                    transcript,
                    records
                        .iter()
                        .map(|value| format!("{value}\n"))
                        .collect::<String>(),
                )
                .unwrap();
            }
            "ambiguous" => {
                let mut claim = rows[0].clone();
                claim["op"] = serde_json::json!("55555555-5555-4555-8555-555555555555");
                let mut terminal = rows[1].clone();
                terminal["op"] = claim["op"].clone();
                terminal["claim_sha256"] = serde_json::json!(digest(claim.to_string().as_bytes()));
                rows.extend([claim, terminal]);
            }
            _ => unreachable!(),
        }
        if mode != "missing" {
            fs::write(
                &ledger,
                rows.iter()
                    .map(|value| format!("{value}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        }
        for direction in [
            HistoryMessageDirection::Forward,
            HistoryMessageDirection::Latest,
        ] {
            let page = catalog
                .messages_with_parts(&id, None, 100, direction, true)
                .unwrap();
            assert_eq!(
                page.messages
                    .iter()
                    .map(|message| message.role.as_str())
                    .collect::<Vec<_>>(),
                ["user", "assistant"],
                "{mode}"
            );
        }
    }
}

#[test]
fn native_notice_history_binding_survives_managed_session_removal() {
    let temp = tempfile::TempDir::new().unwrap();
    let text = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nretained";
    let (catalog, id, _) = notice_catalog(temp.path(), text);
    let sessions = temp.path().join("state/sessions");
    fs::create_dir_all(sessions.join("modeled")).unwrap();
    fs::write(sessions.join("modeled/owned-state"), "synthetic").unwrap();
    fs::remove_dir_all(sessions).unwrap();
    catalog.invalidate();
    let page = catalog
        .messages(&id, None, 100, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(page.messages[1].role, "notice");
    assert_eq!(page.messages[1].text, text);
}

#[test]
fn dense_latest_page_stops_after_one_reverse_chunk() {
    let session = history_session("dense", "2026-09-01T00:00:00Z");
    let mut transcript = vec![b'x'; REVERSE_MESSAGE_CHUNK_BYTES * 2];
    transcript.push(b'\n');
    for index in 0..50 {
        transcript.extend_from_slice(
            format!("{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"message {index}\"}}]}}}}\n").as_bytes(),
        );
    }
    let file_len = transcript.len() as u64;
    let mut reader = CountingReader {
        inner: Cursor::new(transcript),
        bytes_read: 0,
    };
    let page = history_images::read_messages_reverse_from_reader(
        &session,
        &mut reader,
        file_len,
        None,
        50,
        Instant::now() + MESSAGE_SCAN_MAX_DURATION,
        false,
        &history_images::native_notices::Bindings::default(),
    )
    .unwrap();
    assert_eq!(page.messages.len(), 50);
    assert!(reader.bytes_read <= REVERSE_MESSAGE_CHUNK_BYTES);
}

#[test]
fn history_previews_skip_classified_provider_context_notice() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sessions/2026/08/31");
    fs::create_dir_all(&root).unwrap();
    let rows = [
        serde_json::json!({"timestamp":"2026-08-31T00:00:00Z","type":"session_meta","payload":{"id":"abc","cwd":"/work/example","source":"cli","timestamp":"2026-08-31T00:00:00Z"}}),
        serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>\ncontext\n</environment_context>"}}),
        serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"real prompt"}}),
        serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>\ncontext\n</environment_context>"}}),
    ];
    fs::write(
        root.join("rollout.jsonl"),
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let sources = [HistorySource {
        provider: "codex".into(),
        agent_profile: None,
        root: tmp.path().join("sessions"),
    }];
    let page = list(
        &sources,
        HistoryRoots {
            archives: &tmp.path().join("archives"),
            stars: &tmp.path().join("stars"),
        },
        "test",
        None,
        None,
        None,
        25,
    )
    .unwrap();
    let session = &page.sessions[0];
    assert_eq!(
        session.first_user_prompt_preview.as_deref(),
        Some("real prompt")
    );
    assert_eq!(session.prompt_preview.as_deref(), Some("real prompt"));
    assert_eq!(
        session.last_user_prompt_preview.as_deref(),
        Some("real prompt")
    );
}

#[test]
fn history_latest_skips_notice_snapshot_after_real_user_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects/example");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let notice = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nMailbox wake";
    let human = serde_json::json!({"type":"user","sessionId":id,"cwd":"/work/example","message":{"role":"user","content":"real prompt"}});
    let injected = serde_json::json!({"type":"user","sessionId":id,"message":{"role":"user","content":notice}});
    let snapshot = serde_json::json!({"type":"last-prompt","sessionId":id,"lastPrompt":notice});
    fs::write(
        root.join(format!("{id}.jsonl")),
        format!(
            "{human}\n{}{snapshot}\n",
            format!("{injected}\n").repeat(40)
        ),
    )
    .unwrap();
    let sources = [HistorySource {
        provider: "claude".into(),
        agent_profile: None,
        root: tmp.path().join("projects"),
    }];
    let page = list(
        &sources,
        HistoryRoots {
            archives: &tmp.path().join("archives"),
            stars: &tmp.path().join("stars"),
        },
        "test",
        None,
        None,
        None,
        25,
    )
    .unwrap();
    let session = &page.sessions[0];
    assert_eq!(
        session.first_user_prompt_preview.as_deref(),
        Some("real prompt")
    );
    assert_eq!(session.prompt_preview.as_deref(), Some("real prompt"));
    assert_eq!(
        session.last_user_prompt_preview.as_deref(),
        Some("real prompt")
    );
}
