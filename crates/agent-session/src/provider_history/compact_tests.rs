use super::*;
use pretty_assertions::assert_eq;

const TRANSCRIPT: &str = include_str!("../../tests/fixtures/history-parts/claude-compact.jsonl");

fn user_text(messages: &[HistoryMessage]) -> Vec<&str> {
    messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.text.as_str())
        .collect()
}

#[test]
fn claude_compact_fixture_keeps_one_entry_per_compaction_across_history_pages() {
    for (line_count, expected) in [
        (5, vec!["/compact keep focus"]),
        (
            TRANSCRIPT.lines().count(),
            vec![
                "/compact keep focus",
                "/compact keep focus",
                "continue",
                "continue",
                "/compactness is an ordinary prompt",
                "/model",
                "/model",
            ],
        ),
    ] {
        let transcript = TRANSCRIPT
            .lines()
            .take(line_count)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects/repo");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("compact-fixture.jsonl"), &transcript).unwrap();
        let catalog = HistoryCatalog::new(
            vec![HistorySource {
                provider: "claude".into(),
                agent_profile: None,
                root: tmp.path().join("projects"),
            }],
            tmp.path().join("archives"),
            tmp.path().join("stars"),
        );
        let id = stable_history_id("claude", None, "compact-fixture");
        for include_parts in [false, true] {
            for reverse in [false, true] {
                let mut cursor = None;
                let mut messages = Vec::new();
                loop {
                    let direction = match (reverse, cursor.is_some()) {
                        (false, _) => HistoryMessageDirection::Forward,
                        (true, false) => HistoryMessageDirection::Latest,
                        (true, true) => HistoryMessageDirection::Older,
                    };
                    let mut page = catalog
                        .messages_with_parts(&id, cursor.as_deref(), 1, direction, include_parts)
                        .unwrap();
                    if reverse {
                        page.messages.extend(messages);
                        messages = page.messages;
                        cursor = page.older_cursor;
                    } else {
                        messages.extend(page.messages);
                        cursor = page.next_cursor;
                    }
                    if cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(
                    user_text(&messages),
                    expected,
                    "parts={include_parts}, reverse={reverse}"
                );
            }
        }

        let session = catalog.resolve_fresh(&id).unwrap();
        let mut messages = Vec::new();
        for (offset, line) in transcript.lines().enumerate() {
            push_incremental_message(&session, line.as_bytes(), offset as u64, &mut messages);
        }
        assert_eq!(user_text(&messages), expected, "incremental");
    }
}
