//! History route placement and authorization of mailbox read events.
use super::*;
use axum::http::Request;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

const SESSION: &str = "0a1b2c3d-0000-4000-8000-000000000001";
const TOKEN: &str = "test-events-token";

fn row(kind: &str, text: &str) -> String {
    format!(
        "{}\n",
        json!({"type": kind, "sessionId": SESSION, "cwd": "/work/example",
               "message": {"role": kind, "content": [{"type": "text", "text": text}]}})
    )
}

fn ledger_line(id: &str, anchor: u64, body: &str) -> String {
    format!(
        "{}\n",
        json!({"op": "create", "message_id": id, "sender": {"kind": "session", "machine": "machine-a", "session_id": "sender-1"},
               "category": "handoff", "body": body, "anchor": anchor, "created_at_ms": 1_793_000_000_000_i64, "acknowledged": false})
    )
}

/// Three provider rows; events anchored at the start, mid-file and past the end.
async fn app(temp: &Path) -> (Router, String, [String; 3]) {
    let rows = [
        row("user", "first"),
        row("assistant", "second"),
        row("user", "third"),
    ];
    let directory = temp.join("provider/project");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("{SESSION}.jsonl")), rows.concat()).unwrap();
    let offsets = [
        0,
        rows[0].len() as u64,
        (rows[0].len() + rows[1].len()) as u64,
    ];
    let history = provider_history::stable_history_id("claude", None, SESSION);
    let ledger = temp.join(format!("history/mailbox-reads/{history}.jsonl"));
    fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    fs::write(
        ledger,
        [
            ledger_line(
                "11111111-1111-4111-8111-111111111111",
                0,
                "before everything\n",
            ),
            ledger_line(
                "22222222-2222-4222-8222-222222222222",
                offsets[1],
                "between one and two\n",
            ),
            ledger_line(
                "33333333-3333-4333-8333-333333333333",
                9_999,
                "after everything\n",
            ),
        ]
        .concat(),
    )
    .unwrap();
    let mut state = super::super::tests::state(temp, Some(TOKEN), PathBuf::from("/unused-tmux"));
    Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(
        HistoryCatalog::new(
            vec![provider_history::HistorySource {
                provider: "claude".into(),
                agent_profile: None,
                root: temp.join("provider"),
            }],
            provider_history::archive_root(temp),
            provider_history::star_root(temp),
        )
        .with_session_state_dir(temp.to_path_buf()),
    );
    let ids = [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        "33333333-3333-4333-8333-333333333333",
    ]
    .map(|id| format!("{history}-{id}-mailbox"));
    (super::super::router(state), history, ids)
}

async fn get(app: &Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(uri);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn texts(page: &Value) -> Vec<String> {
    page["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            m["text"]
                .as_str()
                .unwrap()
                .rsplit("\n\n")
                .next()
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn pages_in_every_direction_place_events_by_transcript_anchor() {
    let temp = tempfile::TempDir::new().unwrap();
    let (app, history, ids) = app(temp.path()).await;
    let expected = [
        "before everything",
        "first",
        "between one and two",
        "second",
        "third",
        "after everything",
    ];

    // Forward, one provider row per page.
    let (mut seen, mut cursor) = (Vec::new(), String::new());
    for _ in 0..8 {
        let uri = format!("/history/sessions/{history}/messages?limit=1{cursor}");
        let (status, page) = get(&app, &uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        seen.extend(texts(&page));
        match page["data"]["next_cursor"].as_str() {
            Some(next) => cursor = format!("&cursor={next}"),
            None => break,
        }
    }
    assert_eq!(seen, expected);

    // Latest, then older pages.
    let (mut seen, mut next) = (Vec::new(), None::<String>);
    for _ in 0..8 {
        let uri = match &next {
            None => format!("/history/sessions/{history}/messages?limit=1&direction=latest"),
            Some(cursor) => format!(
                "/history/sessions/{history}/messages?limit=1&direction=older&cursor={cursor}"
            ),
        };
        let (status, page) = get(&app, &uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        seen.splice(0..0, texts(&page));
        next = page["data"]["older_cursor"].as_str().map(str::to_string);
        if next.is_none() {
            break;
        }
    }
    assert_eq!(seen, expected);

    let (_, page) = get(
        &app,
        &format!("/history/sessions/{history}/messages?limit=100&parts=v1"),
        Some(TOKEN),
    )
    .await;
    let events: Vec<_> = page["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["mailbox"].is_object())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(events[0]["role"], "notice");
    assert_eq!(events[0]["parts"], json!([]));
    assert_eq!(events[0]["mailbox"]["state"], "read");
    assert_eq!(events[0]["mailbox"]["availability"], "complete");
}

#[tokio::test]
async fn an_unauthenticated_reader_gets_no_event_text() {
    let temp = tempfile::TempDir::new().unwrap();
    let (app, history, _) = app(temp.path()).await;
    for token in [None, Some("wrong-token")] {
        let (status, page) = get(
            &app,
            &format!("/history/sessions/{history}/messages"),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!page.to_string().contains("everything"));
    }
    let other = provider_history::stable_history_id("claude", None, "another-conversation");
    let (status, _) = get(
        &app,
        &format!("/history/sessions/{other}/messages"),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
