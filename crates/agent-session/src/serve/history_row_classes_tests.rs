use super::*;
use axum::http::Request;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

const FIXTURE: &str = include_str!("../../docs/specs/fixtures/history-row-classes-v1.json");
// Shared design merged at f1cacbf8 before this implementation.
const TOKEN: &str = "history-class-test-token";

struct Fixture {
    _temp: tempfile::TempDir,
    app: Router,
    catalog: Arc<HistoryCatalog>,
    file: PathBuf,
    history: String,
}

impl Fixture {
    fn new(provider: &str, records: &[Value]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("history");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("synthetic.jsonl");
        let meta = if provider == "codex" {
            json!({"type":"session_meta","payload":{"id":"synthetic","cwd":"/workspace/fixture","source":"cli","timestamp":"2026-10-01T00:00:00Z"}})
        } else {
            json!({"type":"user","sessionId":"synthetic","cwd":"/workspace/fixture","timestamp":"2026-10-01T00:00:00Z","message":{"role":"user","content":"human opening"}})
        };
        let text = std::iter::once(&meta)
            .chain(records)
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(&file, text).unwrap();
        let mut state =
            super::super::tests::state(temp.path(), Some(TOKEN), PathBuf::from("/unused-tmux"));
        let catalog = Arc::new(HistoryCatalog::new(
            vec![HistorySource {
                provider: provider.into(),
                agent_profile: None,
                root,
            }],
            provider_history::archive_root(temp.path()),
            provider_history::star_root(temp.path()),
        ));
        Arc::get_mut(&mut state).unwrap().history_catalog = catalog.clone();
        Self {
            app: super::super::router(state),
            catalog,
            history: provider_history::stable_history_id(provider, None, "synthetic"),
            file,
            _temp: temp,
        }
    }

    async fn page(&self, query: &str) -> Value {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/history/sessions/{}/messages?{query}",
                        self.history
                    ))
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice::<Value>(
            &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()["data"]
            .clone()
    }

    async fn all(&self, reverse: bool, parts: bool) -> Vec<Value> {
        let mut cursor = None;
        let mut rows = Vec::new();
        for _ in 0..200 {
            let direction = if !reverse {
                "forward"
            } else if cursor.is_none() {
                "latest"
            } else {
                "older"
            };
            let query = format!(
                "direction={direction}&limit=1{}{}",
                if parts { "&parts=v1" } else { "" },
                cursor
                    .as_ref()
                    .map(|c| format!("&cursor={c}"))
                    .unwrap_or_default()
            );
            let page = self.page(&query).await;
            let mut messages = page["messages"].as_array().unwrap().clone();
            if reverse {
                messages.extend(rows);
                rows = messages;
            } else {
                rows.extend(messages);
            }
            cursor = page[if reverse {
                "older_cursor"
            } else {
                "next_cursor"
            }]
            .as_str()
            .map(str::to_owned);
            if cursor.is_none() {
                return rows;
            }
        }
        panic!("paging failed to advance");
    }
}

fn user(provider: &str, text: &str) -> Value {
    if provider == "claude" {
        json!({"type":"user","sessionId":"synthetic","message":{"role":"user","content":text}})
    } else {
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}})
    }
}

fn command(provider: &str, text: &str) -> Value {
    if provider == "claude" {
        let (token, args) = text.split_once(' ').unwrap_or((text, ""));
        user(
            provider,
            &format!(
                "<command-name>{token}</command-name>\n<command-message>{}</command-message>\n<command-args>{args}</command-args>",
                token.trim_start_matches('/')
            ),
        )
    } else {
        user(provider, text)
    }
}

#[tokio::test]
async fn runtime_commands_follow_the_pinned_fixture_through_http_paging() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    for provider in ["claude", "codex"] {
        let cases = fixture["runtime_commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["provider"] == provider)
            .collect::<Vec<_>>();
        let mut records = cases
            .iter()
            .map(|case| command(provider, case["input"].as_str().unwrap()))
            .collect::<Vec<_>>();
        records.push(command(provider, "/model keep focus"));
        let unknown = fixture["unclassified"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                case["input"]
                    .as_str()
                    .is_some_and(|text| text.trim_start().starts_with('/'))
                    && case["expected"]["role"] == "user"
            })
            .filter_map(|case| case["input"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        records.extend(unknown.iter().map(|text| user(provider, text)));
        records.push(if provider == "claude" {json!({"type":"assistant","sessionId":"synthetic","message":{"role":"assistant","content":"/compact"}})} else {json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"/compact"}]}})});

        let f = Fixture::new(provider, &records);
        let before = fs::read(&f.file).unwrap();
        for parts in [false, true] {
            let rows = f.all(false, parts).await;
            assert_eq!(
                rows,
                f.all(true, parts).await,
                "provider={provider}, parts={parts}"
            );
            let commands = rows
                .iter()
                .filter(|row| row["row_class"] == "runtime_command")
                .collect::<Vec<_>>();
            assert_eq!(commands.len(), cases.len() + 1, "provider={provider}");
            for (row, case) in commands.iter().zip(&cases) {
                assert_eq!(row["text"], case["expected"]["text"]);
                assert_eq!(row["role"], "user");
                assert!(row.get("compaction").is_none());
            }
            assert!(
                rows.iter()
                    .filter(|row| row["role"] == "assistant")
                    .all(|row| row.get("row_class").is_none())
            );
            for text in &unknown {
                let row = rows
                    .iter()
                    .find(|row| row["text"].as_str() == Some(text.as_str()))
                    .unwrap();
                assert!(row.get("row_class").is_none(), "{row}");
            }
        }
        let incremental = f
            .catalog
            .incremental_messages(&f.history, None, 16 * 1024 * 1024)
            .unwrap();
        let live = incremental
            .messages
            .iter()
            .filter(|row| row.row_class.is_some())
            .map(|row| serde_json::to_value(row).unwrap())
            .collect::<Vec<_>>();
        assert!(
            incremental
                .messages
                .iter()
                .filter(|row| row.row_class.is_some())
                .all(|row| !row.human_prompt)
        );
        let forward = f
            .all(false, false)
            .await
            .into_iter()
            .filter(|row| row.get("row_class").is_some())
            .collect::<Vec<_>>();
        assert_eq!(live, forward);
        assert_eq!(fs::read(&f.file).unwrap(), before);
    }
}

#[tokio::test]
async fn one_and_two_compactions_keep_boundary_identity_and_summary_in_every_direction() {
    for provider in ["claude", "codex"] {
        for count in [1, 2] {
            let mut records = Vec::new();
            for _ in 0..count {
                records.push(user(provider, "/compact keep focus"));
                if provider == "claude" {
                    records.push(json!({"type":"system","subtype":"compact_boundary","timestamp":"2026-10-01T00:00:01Z"}));
                    records.push(json!({"type":"queue-operation","operation":"remove"}));
                    records.push(json!({"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"role":"user","content":"  summary 🌱\r\n"}}));
                    records.push(command(provider, "/compact keep focus"));
                } else {
                    records.push(json!({"type":"compacted","timestamp":"2026-10-01T00:00:01Z","payload":{"message":"  summary 🌱\r\n"}}));
                }
            }
            let f = Fixture::new(provider, &records);
            let incremental = f
                .catalog
                .incremental_messages(&f.history, None, 16 * 1024 * 1024)
                .unwrap();
            let live = incremental
                .messages
                .iter()
                .filter(|row| row.row_class.is_some())
                .map(|row| serde_json::to_value(row).unwrap())
                .collect::<Vec<_>>();
            assert!(
                incremental
                    .messages
                    .iter()
                    .filter(|row| row.row_class.is_some())
                    .all(|row| !row.human_prompt)
            );
            let history_rows = f
                .all(false, false)
                .await
                .into_iter()
                .filter(|row| row.get("row_class").is_some())
                .collect::<Vec<_>>();
            assert_eq!(live, history_rows);
            let bytes = fs::read_to_string(&f.file).unwrap();
            let offsets = bytes
                .split_inclusive('\n')
                .scan(0, |at, line| {
                    let offset = *at;
                    *at += line.len();
                    Some((offset, line))
                })
                .filter(|(_, line)| {
                    line.contains("compact_boundary") || line.contains("\"compacted\"")
                })
                .map(|(offset, _)| format!("{}-{offset}", f.history))
                .collect::<Vec<_>>();
            for parts in [false, true] {
                let rows = f.all(false, parts).await;
                assert_eq!(rows, f.all(true, parts).await, "provider={provider}");
                let events = rows
                    .iter()
                    .filter(|row| row["row_class"] == "compaction")
                    .collect::<Vec<_>>();
                assert_eq!(events.len(), count);
                assert_eq!(
                    rows.iter()
                        .filter(|row| row["row_class"] == "runtime_command")
                        .count(),
                    count
                );
                for (row, id) in events.iter().zip(&offsets) {
                    assert_eq!(row["id"], *id);
                    assert_eq!(row["role"], "notice");
                    assert_eq!(row["text"], "  summary 🌱\r\n");
                    assert_eq!(row["compaction"], json!({"summary_truncated":false}));
                    assert_eq!(row["timestamp"], "2026-10-01T00:00:01Z");
                    if parts {
                        assert_eq!(row["parts"], json!([]));
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn compaction_summary_scalar_bound_preserves_whitespace_and_non_bmp_text() {
    for provider in ["claude", "codex"] {
        for length in [65536, 65537] {
            let summary = format!(" {}", "🌱".repeat(length - 1));
            let records = if provider == "claude" {
                vec![
                    json!({"type":"system","subtype":"compact_boundary"}),
                    json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":summary}}),
                ]
            } else {
                vec![json!({"type":"compacted","payload":{"message":summary}})]
            };
            let f = Fixture::new(provider, &records);
            let rows = f.page("direction=forward&limit=100&parts=v1").await;
            let row = rows["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["row_class"] == "compaction")
                .expect("one compaction");
            assert_eq!(row["text"].as_str().unwrap().chars().count(), 65536);
            assert_eq!(row["text"], summary.chars().take(65536).collect::<String>());
            assert_eq!(row["compaction"]["summary_truncated"], length > 65536);
            assert_eq!(f.all(false, true).await, f.all(true, true).await);
        }
    }
}

#[tokio::test]
async fn pending_claude_boundary_waits_for_a_complete_following_logical_record() {
    use std::io::Write;
    let f = Fixture::new(
        "claude",
        &[json!({"type":"system","subtype":"compact_boundary"})],
    );
    let first = f.page("direction=forward&limit=1").await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let pending = f
        .page(&format!("cursor={cursor}&direction=forward&limit=1"))
        .await;
    assert_eq!(pending["messages"], json!([]));
    assert_eq!(pending["next_cursor"], cursor);
    assert_eq!(
        f.page("direction=latest&limit=100").await["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let summary=json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"late summary\n"}}).to_string();
    fs::OpenOptions::new()
        .append(true)
        .open(&f.file)
        .unwrap()
        .write_all(summary.as_bytes())
        .unwrap();
    assert_eq!(
        f.page(&format!("cursor={cursor}&limit=100")).await["messages"],
        json!([])
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&f.file)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let completed = f.page(&format!("cursor={cursor}&limit=100")).await;
    assert_eq!(completed["messages"].as_array().unwrap().len(), 1);
    assert_eq!(completed["messages"][0]["text"], "late summary\n");
    assert_eq!(f.all(false, false).await, f.all(true, false).await);
}

#[tokio::test]
async fn missing_summaries_and_intervening_messages_never_borrow_later_summaries() {
    for provider in ["claude", "codex"] {
        let records = if provider == "claude" {
            vec![
                json!({"type":"system","subtype":"compact_boundary"}),
                user(provider, "ordinary message"),
                json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"unassociated"}}),
                json!({"type":"system","subtype":"compact_boundary"}),
                json!({"type":"system","subtype":"compact_boundary"}),
                user(provider, "end"),
            ]
        } else {
            vec![
                json!({"type":"compacted","payload":{}}),
                json!({"type":"compacted","payload":{"message":17}}),
                json!({"type":"compacted","payload":{"message":""}}),
            ]
        };
        let f = Fixture::new(provider, &records);
        let rows = f.all(false, true).await;
        assert_eq!(rows, f.all(true, true).await);
        let events = rows
            .iter()
            .filter(|row| row["row_class"] == "compaction")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 3);
        for row in events {
            assert_eq!(row["text"], "");
            assert_eq!(row["compaction"], json!({"summary_truncated":false}));
        }
    }
}

#[tokio::test]
async fn claude_adjacent_summary_resolution_is_bounded_independently_of_page_direction() {
    let mut records = vec![json!({"type":"system","subtype":"compact_boundary"})];
    // Bookkeeping spans more than the existing 16 MiB reverse window.
    for _ in 0..17 {
        records.push(json!({"type":"progress","data":"x".repeat(1024*1024-128)}));
    }
    records.push(json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"outside bound"}}));
    records.push(user("claude", "after compaction"));
    let f = Fixture::new("claude", &records);
    // Request at the boundary; no full transcript scan or summary-derived id.
    let prefix = fs::read_to_string(&f.file).unwrap();
    let anchor = prefix.find('\n').unwrap() + 1;
    let forward = f.page(&format!("cursor={anchor}&limit=1")).await;
    let event = &forward["messages"][0];
    assert_eq!(event["row_class"], "compaction");
    assert_eq!(event["text"], "");
    let reverse = f
        .page(&format!(
            "direction=older&cursor={}&limit=1",
            anchor + records[0].to_string().len() + 1
        ))
        .await;
    assert_eq!(reverse["messages"][0], *event);
}

#[tokio::test]
async fn runtime_classification_preserves_tabs_and_respects_provider_admission() {
    for provider in ["claude", "codex"] {
        let text = "/model\t  keep 🌱 focus  \t";
        let mut records = vec![user(provider, text)];
        if provider == "codex" {
            let mut injected = user(provider, "/model");
            injected["payload"]["internal_chat_message_metadata_passthrough"]["content_item_kinds"] =
                json!(["agents_md.instructions"]);
            records.push(injected);
        } else {
            let mut sidechain = user(provider, "/model");
            sidechain["isSidechain"] = json!(true);
            records.push(sidechain);
        }
        let f = Fixture::new(provider, &records);
        let rows = f.all(false, false).await;
        assert_eq!(rows, f.all(true, false).await);
        let commands = rows
            .iter()
            .filter(|row| row["row_class"] == "runtime_command")
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["text"], text);
    }
}

#[tokio::test]
async fn claude_foreign_classified_records_never_enter_history_or_summary_association() {
    let mut records = Vec::new();
    for key in ["sessionId", "session_id"] {
        let mut foreign = command("claude", "/model");
        foreign.as_object_mut().unwrap().remove("sessionId");
        foreign[key] = json!("other");
        records.push(foreign);
        records.push(json!({"type":"system","subtype":"compact_boundary",key:"other"}));
        records.push(json!({"type":"user","isCompactSummary":true,key:"other","message":{"role":"user","content":"foreign summary"}}));
    }
    records.push(json!({"type":"system","subtype":"compact_boundary","sessionId":"synthetic"}));
    records.push(json!({"type":"user","isCompactSummary":true,"sessionId":"other","message":{"role":"user","content":"foreign summary"}}));
    records.push(json!({"type":"user","isCompactSummary":true,"session_id":"synthetic","message":{"role":"user","content":"current summary"}}));
    let f = Fixture::new("claude", &records);
    let rows = f.all(false, false).await;
    assert_eq!(rows, f.all(true, false).await);
    let classified = rows
        .iter()
        .filter(|row| row.get("row_class").is_some())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(classified.len(), 1);
    assert_eq!(classified[0]["row_class"], "compaction");
    assert_eq!(classified[0]["text"], "current summary");
    let incremental = f
        .catalog
        .incremental_messages(&f.history, None, 16 * 1024 * 1024)
        .unwrap();
    let live = incremental
        .messages
        .iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .filter(|row| row.get("row_class").is_some())
        .collect::<Vec<_>>();
    assert_eq!(live, classified);
}

#[tokio::test]
async fn class_contract_long_runtime_argument_text_agrees_with_card_preview() {
    for provider in ["claude", "codex"] {
        for args in ["x".repeat(20 * 1024), "🌱".repeat(65537)] {
            let input = format!("/model {args}");
            let f = Fixture::new(provider, &[user(provider, &input)]);
            let page = f.page("limit=100").await;
            let row = page["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["row_class"] == "runtime_command")
                .unwrap();
            let source = crate::provider_prompt::ProviderPromptSource::for_history(
                provider,
                "synthetic",
                f.file.clone(),
            )
            .unwrap();
            let mut tracker =
                crate::provider_prompt::ProviderLastPromptTracker::open_source(source)
                    .expect("exact fixture source");
            tracker.refresh();
            let card = serde_json::to_value(tracker.last_message().unwrap()).unwrap();
            assert_eq!(card["row_class"], row["row_class"]);
            assert_eq!(card["text"], row["text"]);
        }
    }
}

#[tokio::test]
async fn class_contract_extended_boundary_stays_pending_then_emits_once_incrementally() {
    use std::io::Write;
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary","padding":"x".repeat(1024*1024+4096)}),
        ],
    );
    let anchor = fs::read_to_string(&f.file).unwrap().find('\n').unwrap() as u64 + 1;
    let chunk = crate::retitle_v3::REFRESH_CHUNK_BYTES;
    let first = f
        .catalog
        .incremental_messages(&f.history, None, chunk)
        .unwrap();
    let pending = f
        .catalog
        .incremental_messages(&f.history, Some(&first.cursor), chunk)
        .unwrap();
    assert_eq!(pending.cursor.offset, anchor);
    assert!(pending.messages.is_empty());
    let summary = json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"completed"}});
    writeln!(
        fs::OpenOptions::new().append(true).open(&f.file).unwrap(),
        "{summary}"
    )
    .unwrap();
    let complete = f
        .catalog
        .incremental_messages(&f.history, Some(&pending.cursor), chunk)
        .unwrap();
    let tail = f
        .catalog
        .incremental_messages(&f.history, Some(&complete.cursor), chunk)
        .unwrap();
    let live = complete
        .messages
        .iter()
        .chain(&tail.messages)
        .map(|row| serde_json::to_value(row).unwrap())
        .filter(|row| row.get("row_class").is_some())
        .collect::<Vec<_>>();
    let history = f
        .all(false, false)
        .await
        .into_iter()
        .filter(|row| row.get("row_class").is_some())
        .collect::<Vec<_>>();
    assert_eq!(history.len(), 1);
    assert_eq!(live, history);
    assert_eq!(live[0]["id"], format!("{}-{anchor}", f.history));
    assert_eq!(live[0]["text"], "completed");
}

#[tokio::test]
async fn queued_runtime_commands_do_not_become_first_history_previews() {
    for markup in [false, true] {
        for blocks in [false, true] {
            let command = if markup {
                "<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>keep focus</command-args>"
            } else {
                "/model keep focus"
            };
            let prompt = if blocks {
                json!([{"type":"text","text":command}])
            } else {
                json!(command)
            };
            let records = [
                json!({"type":"attachment","sessionId":"synthetic","cwd":"/workspace/fixture","timestamp":"2026-10-01T00:00:00Z","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},"prompt":prompt}}),
                user("claude", "first ordinary request"),
            ];
            let f = Fixture::new("claude", &[]);
            // This session begins with a queued command, before any human prompt.
            fs::write(
                &f.file,
                records
                    .iter()
                    .map(|row| format!("{row}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            let rows = f.all(false, false).await;
            if markup {
                // Queued command markup is an existing machine-prompt shape.
                assert_eq!(rows.len(), 1);
            } else {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0]["row_class"], "runtime_command");
                assert_eq!(rows[0]["text"], "/model keep focus");
            }
            let human = rows.last().unwrap();
            assert_eq!(human["text"], "first ordinary request");
            assert!(human.get("row_class").is_none());

            let response = f
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/history/sessions")
                        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let sessions = body["data"]["sessions"].as_array().unwrap();
            let session = sessions
                .iter()
                .find(|session| session["id"] == f.history)
                .unwrap();
            for field in ["first_user_prompt_preview", "prompt_preview"] {
                assert_eq!(
                    session[field], "first ordinary request",
                    "markup={markup}, blocks={blocks}, field={field}"
                );
            }
        }
    }
}

#[tokio::test]
async fn unmarked_notices_match_history_paging_and_card() {
    for provider in ["claude", "codex"] {
        let mut cases = vec![
            ("※ Mailbox：body", true),
            ("※ Unknown：body", false),
            ("Mailbox：body", false),
            ("※ Mailbox:body", false),
            ("prefix ※ Mailbox：body", false),
            ("※ Mailbox：", false),
            (
                "Mailbox: 2026-11-02T01:02:03.5Z; agent-session message inbox",
                false,
            ),
            ("Quoted: Mailbox: agent-session message inbox", false),
            (
                "Coordination mailbox has unread messages; run agent-session message inbox.",
                true,
            ),
            ("壓縮完成；執行自己角色的 session-start，接著續做。", true),
        ];
        if provider == "codex" {
            cases.push(("Mailbox: agent-session message inbox", true));
            cases.push(("Mailbox: b=1 h=2", true));
            cases.push((
                "<environment_context>\ncontext\n</environment_context>",
                true,
            ));
            cases.push((
                "# AGENTS.md instructions for project\n\n<INSTRUCTIONS>\ncontent\n</INSTRUCTIONS>",
                true,
            ));
        }
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let fixture_cases = fixture["notice_recognition"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                case["providers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p == provider)
            })
            .map(|case| (case["input"].as_str().unwrap(), true));
        cases.extend(fixture_cases);
        cases.extend(
            fixture["unclassified"]
                .as_array()
                .unwrap()
                .iter()
                .skip(5)
                .take(24)
                .filter(|case| provider != "claude" || case["name"] != "unknown-22")
                .map(|case| (case["input"].as_str().unwrap(), false)),
        );
        for (text, notice) in cases {
            if provider == "claude"
                && !notice
                && crate::coordination::is_mailbox_reminder_prompt(text)
            {
                continue;
            }
            let mut record = user(provider, text);
            if provider == "claude" {
                record["version"] = json!("2.1.3");
            }
            if provider == "codex" && notice && text.starts_with(['<', '#']) {
                record["payload"]
                    .as_object_mut()
                    .unwrap()
                    .remove("internal_chat_message_metadata_passthrough");
            }
            let f = Fixture::new(provider, &[record]);
            let rows = f.all(false, false).await;
            assert_eq!(rows, f.all(true, false).await);
            let row = rows
                .iter()
                .find(|row| row["text"] == text)
                .unwrap_or_else(|| panic!("missing {provider}: {text:?}"));
            let parts = f.all(false, true).await;
            let with_parts = parts.iter().find(|row| row["text"] == text).unwrap();
            if provider == "claude" {
                assert!(
                    !with_parts["parts"].as_array().unwrap().is_empty(),
                    "{provider}: {text:?}"
                );
            }
            assert_eq!(row["row_class"] == "notice", notice, "{provider}: {text}");
            let source = crate::provider_prompt::ProviderPromptSource::for_history(
                provider,
                "synthetic",
                f.file.clone(),
            )
            .unwrap();
            let mut tracker =
                crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
            tracker.refresh();
            let card = serde_json::to_value(tracker.last_message().unwrap()).unwrap();
            assert_eq!(card["text"], row["text"]);
            assert_eq!(card["row_class"], row["row_class"]);
        }
    }
}
