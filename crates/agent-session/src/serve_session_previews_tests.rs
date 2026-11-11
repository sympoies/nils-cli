use super::*;
use axum::http::Request;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;

const TOKEN: &str = "previews-test-token";
const PROVIDER_ID: &str = "11111111-1111-4111-8111-111111111111";
const NOTICE_TEXT: &str = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nlaunch body";

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A Claude transcript with a human prompt, a header-only user paste, a
/// receipt-bound launch notice row and an assistant reply. The producer ledger is a
/// modeled fixture, never evidence of native execution.
fn receipt_bound_catalog(root: &Path) -> (Arc<HistoryCatalog>, String) {
    let row_id = "22222222-2222-4222-8222-222222222222";
    let op = "33333333-3333-4333-8333-333333333333";
    let history_id = provider_history::stable_history_id("claude", None, PROVIDER_ID);
    let directory = root.join("provider/project");
    fs::create_dir_all(&directory).unwrap();
    let user = |uuid: &str, text: &str| {
        json!({"type":"user","sessionId":PROVIDER_ID,"uuid":uuid,"cwd":"/work/example",
        "promptSource":"typed","version":"2.1.296","message":{"role":"user","content":text}})
    };
    let human_row = format!(
        "{}\n",
        user("44444444-4444-4444-8444-444444444444", "human words")
    );
    let paste_row = format!(
        "{}\n",
        user("55555555-5555-4555-8555-555555555555", "※ Mailbox：")
    );
    let notice_row = json!({"type":"user","sessionId":PROVIDER_ID,"uuid":row_id,
        "cwd":"/work/example","promptSource":"system","isSidechain":false,
        "version":"2.1.296","timestamp":"2026-09-01T00:00:00Z",
        "origin":{"kind":"plugin","name":"agent-session-claude-notice","asUser":true},
        "message":{"role":"user","content":[{"type":"text","text":NOTICE_TEXT}]}})
    .to_string();
    let assistant = json!({"type":"assistant","sessionId":PROVIDER_ID,
        "message":{"role":"assistant","content":[{"type":"text","text":"response"}]}});
    let prefix = format!("{human_row}{paste_row}");
    fs::write(
        directory.join(format!("{PROVIDER_ID}.jsonl")),
        format!("{prefix}{notice_row}\n{assistant}\n"),
    )
    .unwrap();
    let binding = json!({"launch":{"session_id":"modeled","incarnation":"runtime-one",
        "generation":1,"profile":null},"provider":"claude","provider_session_id":PROVIDER_ID,
        "provider_version":"2.1.296","module_sha256":"a".repeat(64)});
    let claim = json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"claim","op":op,"binding":binding,"history_id":history_id,
        "envelope_bytes":NOTICE_TEXT.len(),"envelope_sha256":digest(NOTICE_TEXT.as_bytes()),
        "original_budget_ms":15000,
        "baseline":{"device":0,"inode":0,"eof":prefix.len(),
        "continuity_sha256":digest(prefix.as_bytes())}})
    .to_string();
    let terminal = json!({"schema_version":"agent-session.claude-notice-ledger.v1",
        "kind":"terminal","op":op,"claim_sha256":digest(claim.as_bytes()),
        "state":"submitted","native_uuid":row_id,"row_offset":prefix.len(),
        "row_payload_sha256":digest(notice_row.as_bytes()),
        "envelope_sha256":digest(NOTICE_TEXT.as_bytes()),
        "timestamp":"2026-09-01T00:00:00Z","confirmation_elapsed_ms":10});
    let state = root.join("state");
    let ledger_dir = state.join("history/claude-notices");
    fs::create_dir_all(&ledger_dir).unwrap();
    fs::set_permissions(&ledger_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = ledger_dir.join("ledger.jsonl");
    fs::write(&ledger, format!("{claim}\n{terminal}\n")).unwrap();
    fs::set_permissions(ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: root.join("provider"),
        }],
        provider_history::archive_root(&state),
        provider_history::star_root(&state),
    )
    .with_session_state_dir(state);
    (Arc::new(catalog), history_id)
}

async fn serve_rows(app: &Router, history: &str, direction: &str, parts: bool) -> Vec<Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/history/sessions/{history}/messages?direction={direction}&limit=50{}",
                    if parts { "&parts=v1" } else { "" }
                ))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()["data"]["messages"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn history_rows_equal_the_serve_route_for_receipt_and_header_only_rows() {
    let temp = tempfile::tempdir().unwrap();
    let (catalog, history) = receipt_bound_catalog(temp.path());
    let mut state =
        super::super::tests::state(temp.path(), Some(TOKEN), PathBuf::from("/unused-tmux"));
    Arc::get_mut(&mut state).unwrap().history_catalog = catalog.clone();
    let app = super::super::router(state);
    for parts in [false, true] {
        let (start, latest) = history_rows(&catalog, &history, 50, parts).unwrap();
        assert_eq!(start, serve_rows(&app, &history, "forward", parts).await);
        assert_eq!(latest, serve_rows(&app, &history, "latest", parts).await);
        let roles = start
            .iter()
            .map(|row| (row["role"].as_str().unwrap(), row["text"].as_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("user", "human words"),
                ("user", "※ Mailbox："),
                ("notice", NOTICE_TEXT),
                ("assistant", "response"),
            ]
        );
    }
}

fn fake_tmux(dir: &Path, state_dir: &Path, id: &str) -> PathBuf {
    let bin = dir.join("fake-tmux");
    fs::write(
        &bin,
        format!(
            r#"#!/usr/bin/env sh
case "$1" in
  display-message) printf '$88\t%%88\t77\n'; exit 0 ;;
  show-environment)
    case "$4" in
      AGENT_SESSION_ID) printf 'AGENT_SESSION_ID=%s\n' {id} ;;
      AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state_dir} ;;
      AGENT_SESSION_RUNTIME_ID) printf 'AGENT_SESSION_RUNTIME_ID=launch-%s\n' {id} ;;
    esac
    exit 0 ;;
  has-session) exit 0 ;;
  *) exit 42 ;;
esac
"#,
            id = shell_words::quote(id),
            state_dir = shell_words::quote(&state_dir.to_string_lossy()),
        ),
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_last_prompt_equals_the_serve_list_projection() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("state");
    let codex_home = temp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let provider_id = "previews-parity";
    let lines = [
        json!({"type":"session_meta","payload":{"id":provider_id,"session_id":provider_id,
            "cwd":"/tmp","source":"cli","timestamp":"2099-01-01T00:00:00Z"}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"parity prompt"}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"parity reply"}}),
    ];
    fs::write(
        &transcript,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let id = "previews-session";
    let dir = state_dir.join("sessions").join(id);
    fs::create_dir_all(&dir).unwrap();
    let record = json!({"schema_version":"agent-session.session.v1","id":id,"agent":"codex",
        "mode":"interactive","title":null,"cwd":"/tmp","tmux_session":"hs-previews",
        "prompt_file":null,"log_file":null,"created_at":"2000-01-01T00:00:00Z",
        "updated_at":"2000-01-01T00:00:00Z",
        "runtime":{"kind":"tmux","tmux_session":"hs-previews","generation":1,
            "started_at":"2000-01-01T00:00:00Z","launch_id":format!("launch-{id}")},
        "provider_resume":{"provider":"codex","session_id":provider_id,
            "captured_at":"2099-01-01T00:00:00Z","capture_method":"codex-explicit-session-id",
            "resume_args":["resume",provider_id]}});
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    let tmux = fake_tmux(temp.path(), &state_dir, id);
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _tmux = EnvGuard::set(&lock, "AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap());
    let _profiles = EnvGuard::remove(&lock, "AGENT_SESSION_LAUNCH_PROFILES");

    let state = super::super::tests::state(&state_dir, Some(TOKEN), tmux);
    let served = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = super::super::router(state.clone())
                .oneshot(
                    Request::builder()
                        .uri("/sessions")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let session =
                serde_json::from_slice::<Value>(&body).unwrap()["data"]["sessions"][0].clone();
            if session["last_prompt_state"] == "current" {
                break session;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("serve projection");

    let context = CliContext {
        state_dir,
        host: None,
    };
    let args = cli::PreviewsArgs {
        id: id.into(),
        rows: 50,
        parts: None,
        format: OutputFormat::Json,
    };
    let result = tokio::task::spawn_blocking(move || collect(&context, &args))
        .await
        .unwrap()
        .unwrap();
    let cli = serde_json::to_value(&result).unwrap();
    assert_eq!(served["last_prompt"]["text"], "parity prompt");
    for field in ["last_prompt", "last_message", "last_prompt_state"] {
        assert_eq!(cli[field], served[field], "{field}");
    }
    assert_eq!(cli["id"], id);
    assert_eq!(
        cli["history_id"],
        provider_history::stable_history_id("codex", None, provider_id)
    );
}
