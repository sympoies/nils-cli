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
async fn one_and_two_compactions_keep_ordered_identity_and_summary_in_every_direction() {
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
                    if provider == "claude" {
                        line.contains("<command-name>")
                    } else {
                        line.contains("\"compacted\"")
                    }
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
            ("※ Mailbox：body", false),
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
                false,
            ),
            ("壓縮完成；執行自己角色的 session-start，接著續做。", false),
        ];
        if provider == "codex" {
            cases.push(("Mailbox: agent-session message inbox", false));
            cases.push(("Mailbox: b=1 h=2", false));
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
            .map(|case| {
                (
                    case["input"].as_str().unwrap(),
                    case["expected"]["row_class"] == "notice",
                )
            });
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
            let expected = if notice {
                text.to_owned()
            } else {
                text.replace("\r\n", "\n")
            };
            assert_eq!(card["text"], expected);
            assert_eq!(card["row_class"], row["row_class"]);
        }
    }
}

#[tokio::test]
async fn notice_preserves_admission_and_stays_out_of_human_previews() {
    let marked = crate::notice::NoticeEnvelope::new("mailbox.wake", "agent-session", None)
        .unwrap()
        .encode("Mailbox: 1 report; agent-session message inbox\nRead: /opt/bin/agent-session message inbox --session recipient --format json\nShow: /opt/bin/agent-session message show --session recipient --message MESSAGE_ID --format json")
        .unwrap();
    for provider in ["claude", "codex"] {
        let mut records = vec![user(provider, &marked), user(provider, "actual human")];
        if provider == "codex" {
            records.push(
                json!({"type":"event_msg","payload":{"type":"user_message","message":marked}}),
            );
        } else {
            records.push(user(provider, &marked));
        }
        let f = Fixture::new(provider, &records);
        if provider == "claude" {
            let lines = fs::read_to_string(&f.file).unwrap();
            let mut rows: Vec<Value> = lines
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            rows[0]["message"]["content"] = json!(marked);
            fs::write(
                &f.file,
                rows.iter()
                    .map(|row| format!("{row}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        }
        let rows = f.all(false, false).await;
        let notice = rows.iter().find(|row| row["text"] == marked).unwrap();
        assert_eq!(notice["row_class"], "notice");
        assert_eq!(notice["text"], marked);
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
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        let session = body["data"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == f.history)
            .unwrap();
        for field in ["first_user_prompt_preview", "prompt_preview"] {
            assert_eq!(session[field], "actual human", "{provider}: {field}");
        }
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
        assert_eq!(card["text"], marked);
        assert_eq!(card["row_class"], "notice");
    }
}

#[tokio::test]
async fn compact_command_precedes_its_boundary_in_both_paging_directions() {
    let records = vec![
        user("claude", "/compact keep focus"),
        json!({"type":"system","subtype":"compact_boundary","timestamp":"2026-10-01T00:00:37Z"}),
        json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
        command("claude", "/compact keep focus"),
        user("claude", "after"),
    ];
    let preview_fixture = Fixture::new("claude", &records[..4]);
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "synthetic",
        preview_fixture.file.clone(),
    )
    .unwrap();
    let mut tracker =
        crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
    tracker.refresh();
    let card = serde_json::to_value(tracker.last_message().unwrap()).unwrap();
    let f = Fixture::new("claude", &records);
    let rows = f.all(false, false).await;
    assert_eq!(rows, f.all(true, false).await);
    let visible = rows
        .iter()
        .map(|row| (row["row_class"].as_str(), row["text"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            (None, "human opening"),
            (Some("runtime_command"), "/compact keep focus"),
            (Some("compaction"), "summary"),
            (None, "after")
        ]
    );
    assert_eq!(card["row_class"], "compaction");
}

#[tokio::test]
async fn codex_compaction_keeps_file_order() {
    let f = Fixture::new(
        "codex",
        &[
            user("codex", "before"),
            json!({"type":"compacted","payload":{"message":"summary"}}),
            command("codex", "/compact"),
            user("codex", "after"),
        ],
    );
    let rows = f.all(false, false).await;
    assert_eq!(rows, f.all(true, false).await);
    let visible = rows
        .iter()
        .map(|row| (row["row_class"].as_str(), row["text"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            (None, "before"),
            (Some("compaction"), "summary"),
            (Some("runtime_command"), "/compact"),
            (None, "after")
        ]
    );
}

#[tokio::test]
async fn compact_post_summary_echo_only_keeps_order_and_identity() {
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary"}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
            command("claude", "/compact keep focus"),
            user("claude", "after"),
        ],
    );
    for parts in [false, true] {
        let rows = f.all(false, parts).await;
        assert_eq!(rows, f.all(true, parts).await);
        assert_eq!(
            rows.iter()
                .map(|r| r["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["human opening", "/compact keep focus", "summary", "after"]
        );
        let contents = fs::read_to_string(&f.file).unwrap();
        let offsets = contents
            .split_inclusive('\n')
            .scan(0usize, |offset, line| {
                let start = *offset;
                *offset += line.len();
                Some(start)
            })
            .collect::<Vec<_>>();
        assert_eq!(rows[1]["id"], format!("{}-{}", f.history, offsets[1]));
        assert_eq!(rows[2]["id"], format!("{}-{}", f.history, offsets[3]));
        // A reverse cursor issued before the ordering fix at the echo remains
        // a valid byte boundary. Restart paging to reconcile an upgrade mid-pair.
        let old_cursor_page = f
            .page(&format!(
                "direction=older&cursor={}&limit=1{}",
                offsets[3],
                if parts { "&parts=v1" } else { "" }
            ))
            .await;
        assert_eq!(old_cursor_page["messages"][0], rows[1]);
    }
}

#[test]
fn compact_post_summary_echo_keeps_compaction_card_preview() {
    use std::io::Write;
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary"}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
        ],
    );
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "synthetic",
        f.file.clone(),
    )
    .unwrap();
    let mut tracker =
        crate::provider_prompt::ProviderLastPromptTracker::open_source(source.clone()).unwrap();
    tracker.refresh();
    writeln!(
        fs::OpenOptions::new().append(true).open(&f.file).unwrap(),
        "{}",
        command("claude", "/compact")
    )
    .unwrap();
    tracker.refresh();
    let card = serde_json::to_value(tracker.last_message().unwrap()).unwrap();
    assert_eq!(card["row_class"], "compaction");
    assert_eq!(card["text"], "summary");
    let mut cold = crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
    cold.refresh();
    assert_eq!(cold.last_message(), tracker.last_message());
}

#[tokio::test]
async fn manual_compact_polling_waits_for_the_echo_without_losing_either_row() {
    use std::io::Write;
    let f = Fixture::new(
        "claude",
        &[
            user("claude", "/compact"),
            json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual","preTokens":1000}}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
        ],
    );
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "synthetic",
        f.file.clone(),
    )
    .unwrap();
    let mut tracker =
        crate::provider_prompt::ProviderLastPromptTracker::open_source(source.clone()).unwrap();
    tracker.refresh();
    assert_eq!(tracker.last_message().unwrap().text, "human opening");
    let first = f.page("limit=1").await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let pending = f.page(&format!("cursor={cursor}&limit=1")).await;
    assert_eq!(pending["messages"], json!([]));
    let cursor = pending["next_cursor"].as_str().unwrap();
    let echo = command("claude", "/compact").to_string();
    let mut file = fs::OpenOptions::new().append(true).open(&f.file).unwrap();
    file.write_all(echo.as_bytes()).unwrap();
    tracker.refresh();
    assert_eq!(tracker.last_message().unwrap().text, "human opening");
    assert_eq!(
        f.page(&format!("cursor={cursor}&limit=1")).await["messages"],
        json!([])
    );
    file.write_all(b"\n").unwrap();
    tracker.refresh();
    assert_eq!(tracker.last_message().unwrap().text, "summary");
    let mut cold = crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
    cold.refresh();
    assert_eq!(cold.last_message(), tracker.last_message());
    let command = f.page(&format!("cursor={cursor}&limit=1")).await;
    assert_eq!(command["messages"][0]["row_class"], "runtime_command");
    let compaction = f
        .page(&format!(
            "cursor={}&limit=1",
            command["next_cursor"].as_str().unwrap()
        ))
        .await;
    assert_eq!(compaction["messages"][0]["row_class"], "compaction");
    assert_eq!(compaction["messages"][0]["text"], "summary");
    let end = f
        .page(&format!(
            "cursor={}&limit=1",
            compaction["next_cursor"].as_str().unwrap()
        ))
        .await;
    assert_eq!(end["messages"], json!([]));
    assert_eq!(f.all(false, true).await, f.all(true, true).await);
}

#[tokio::test]
async fn nonmanual_compaction_at_idle_eof_still_publishes_immediately() {
    for trigger in [json!("auto"), json!("unknown"), Value::Null] {
        let f = Fixture::new(
            "claude",
            &[
                json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":trigger}}),
                json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
            ],
        );
        let page = f.page("direction=latest&limit=1").await;
        assert_eq!(page["messages"][0]["row_class"], "compaction");
        assert_eq!(page["messages"][0]["text"], "summary");
    }
}

#[tokio::test]
async fn manual_compaction_without_an_echo_resolves_at_the_next_message() {
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"}}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
            user("claude", "after"),
        ],
    );
    let rows = f.all(false, false).await;
    assert_eq!(rows, f.all(true, false).await);
    assert_eq!(
        rows.iter()
            .map(|row| row["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["human opening", "summary", "after"]
    );
}

#[tokio::test]
async fn automatic_compaction_does_not_borrow_a_later_compact_command() {
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"auto"}}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"automatic summary"}}),
            command("claude", "/compact"),
        ],
    );
    let rows = f.all(false, true).await;
    assert_eq!(rows, f.all(true, true).await);
    assert_eq!(
        rows.iter()
            .map(|row| row["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["human opening", "automatic summary", "/compact"]
    );
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "synthetic",
        f.file.clone(),
    )
    .unwrap();
    let mut tracker =
        crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
    tracker.refresh();
    assert_eq!(
        serde_json::to_value(tracker.last_message().unwrap()).unwrap()["row_class"],
        "runtime_command"
    );
}

#[tokio::test]
async fn compact_pair_repeated_echo_is_hidden_in_history_and_card() {
    use std::io::Write;
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"}}),
            json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
            command("claude", "/compact keep focus"),
        ],
    );
    let before = f.page("limit=3").await;
    let cursor = before["next_cursor"].as_str().unwrap().to_owned();
    let expected_plain = before["messages"].as_array().unwrap().clone();
    let expected_parts = f.all(false, true).await;
    assert_eq!(
        expected_plain
            .iter()
            .map(|row| row["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["human opening", "/compact keep focus", "summary"]
    );
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "synthetic",
        f.file.clone(),
    )
    .unwrap();
    let mut tracker =
        crate::provider_prompt::ProviderLastPromptTracker::open_source(source.clone()).unwrap();
    tracker.refresh();
    let expected_card = tracker.last_message().unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&f.file).unwrap(),
        "{}",
        command("claude", "/compact keep focus")
    )
    .unwrap();

    for (parts, expected) in [(false, expected_plain), (true, expected_parts)] {
        assert_eq!(f.all(false, parts).await, expected);
        assert_eq!(f.all(true, parts).await, expected);
    }
    assert_eq!(
        f.page(&format!("cursor={cursor}&limit=1")).await["messages"],
        json!([])
    );
    tracker.refresh();
    assert_eq!(tracker.last_message().as_ref(), Some(&expected_card));
    let mut cold = crate::provider_prompt::ProviderLastPromptTracker::open_source(source).unwrap();
    cold.refresh();
    assert_eq!(cold.last_message().as_ref(), Some(&expected_card));
}

#[tokio::test]
async fn compact_pair_suppression_keeps_distinct_invocations() {
    let pair = vec![
        json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"}}),
        json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}),
        command("claude", "/compact keep focus"),
        command("claude", "/compact keep focus"),
    ];
    for separate_boundary in [false, true] {
        let mut records = pair.clone();
        if separate_boundary {
            records.extend(pair.clone());
        } else {
            records.push(user("claude", "after"));
            records.push(command("claude", "/compact keep focus"));
        }
        let f = Fixture::new("claude", &records);
        let rows = f.all(false, true).await;
        assert_eq!(rows, f.all(true, true).await);
        assert_eq!(
            rows.iter()
                .filter(|row| row["row_class"] == "runtime_command")
                .map(|row| row["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["/compact keep focus", "/compact keep focus"]
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row["row_class"] == "compaction")
                .count(),
            if separate_boundary { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn claude_2_1_317_compact_caveat_keeps_order_and_monotonic_ids() {
    let f = Fixture::new(
        "claude",
        &[
            json!({"type":"user","uuid":"input","isMeta":false,"timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":"/compact"}}),
            json!({"type":"system","uuid":"boundary","parentUuid":"input","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"},"timestamp":"2026-10-01T00:00:13Z"}),
            json!({"type":"user","uuid":"summary","parentUuid":"boundary","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"timestamp":"2026-10-01T00:00:13Z","message":{"role":"user","content":"summary"}}),
            json!({"type":"user","uuid":"caveat","parentUuid":"summary","isMeta":true,"timestamp":"2026-10-01T00:00:01.006Z","message":{"role":"user","content":"<local-command-caveat>Local command</local-command-caveat>"}}),
            json!({"type":"user","uuid":"echo","parentUuid":"caveat","isMeta":false,"timestamp":"2026-10-01T00:00:01.006Z","message":{"role":"user","content":"<command-name>/compact</command-name>\n<command-message>compact</command-message>\n<command-args></command-args>"}}),
            json!({"type":"user","parentUuid":"echo","message":{"role":"user","content":"<local-command-stdout>done</local-command-stdout>"}}),
            json!({"type":"attachment","attachment":{"type":"fixture"}}),
        ],
    );
    for parts in [false, true] {
        let rows = f.all(false, parts).await;
        assert_eq!(rows, f.all(true, parts).await);
        assert_eq!(rows, f.all(false, parts).await);
        let pair: Vec<_> = rows
            .iter()
            .filter(|r| {
                matches!(
                    r["row_class"].as_str(),
                    Some("runtime_command" | "compaction")
                )
            })
            .collect();
        assert_eq!(
            pair.iter()
                .map(|r| r["row_class"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["runtime_command", "compaction"]
        );
        let offset = |r: &Value| {
            r["id"]
                .as_str()
                .unwrap()
                .rsplit('-')
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        };
        assert!(offset(pair[0]) < offset(pair[1]));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn synthetic_large_compaction_page_measurement() {
    let mut records = Vec::new();
    for _ in 0..150 {
        records.push(json!({"type":"progress","data":"x".repeat(200_000)}));
        records.push(json!({"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"}}));
        records.push(json!({"type":"user","isCompactSummary":true,"message":{"role":"user","content":"summary"}}));
        records.push(command("claude", "/compact"));
    }
    let f = Fixture::new("claude", &records);
    let read_bytes = || -> u64 {
        fs::read_to_string("/proc/thread-self/io")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("rchar: "))
            .unwrap()
            .parse()
            .unwrap()
    };
    let before = read_bytes();
    let started = std::time::Instant::now();
    let page = f
        .catalog
        .messages(
            &f.history,
            None,
            100,
            provider_history::HistoryMessageDirection::Latest,
        )
        .unwrap();
    let bytes = read_bytes() - before;
    let rows = page.messages.len();
    eprintln!(
        "synthetic compaction page: {rows} rows in {:?}",
        started.elapsed()
    );
    eprintln!("synthetic compaction page: {bytes} bytes read");
    assert!(
        bytes < 100_000_000,
        "page repeatedly read the preceding window: {bytes} bytes"
    );
}
