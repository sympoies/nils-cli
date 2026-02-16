use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
use nils_test_support::fs::write_json;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tungstenite::Message;

fn api_websocket_bin() -> PathBuf {
    resolve("api-websocket")
}

fn run_api_websocket(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut options = CmdOptions::default().with_cwd(cwd);
    for key in [
        "WS_URL",
        "WS_ENV_DEFAULT",
        "WS_TOKEN_NAME",
        "WS_HISTORY_ENABLED",
        "WS_HISTORY_FILE",
        "WS_HISTORY_LOG_URL_ENABLED",
        "WS_JWT_VALIDATE_ENABLED",
        "ACCESS_TOKEN",
        "SERVICE_TOKEN",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        options = options.with_env_remove(key);
    }
    options = options.with_env("NO_PROXY", "127.0.0.1,localhost");
    options = options.with_env("no_proxy", "127.0.0.1,localhost");

    for (k, v) in envs {
        options = options.with_env(k, v);
    }

    run_with(&api_websocket_bin(), args, &options)
}

fn spawn_echo_server() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept websocket stream");
        let mut ws = tungstenite::accept(stream).expect("accept websocket handshake");
        loop {
            match ws.read() {
                Ok(Message::Text(text)) => {
                    let response = if text.trim() == "ping" {
                        "{\"ok\":true}".to_string()
                    } else if text.trim() == "plain" {
                        "boom-body".to_string()
                    } else {
                        text.to_string()
                    };
                    ws.send(Message::Text(response)).expect("send response");
                }
                Ok(Message::Close(_)) => {
                    let _ = ws.close(None);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    (format!("ws://{addr}/ws"), handle)
}

fn write_request(path: &Path, send_text: &str, expect: serde_json::Value) {
    write_json(
        path,
        &serde_json::json!({
            "steps": [
                {"type": "send", "text": send_text},
                {"type": "receive", "expect": expect},
                {"type": "close"}
            ]
        }),
    );
}

#[test]
fn call_success_prints_response_and_writes_history() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/websocket")).expect("mkdir setup");
    std::fs::create_dir_all(root.join("requests")).expect("mkdir requests");

    let (url, handle) = spawn_echo_server();
    std::fs::write(
        root.join("setup/websocket/endpoints.env"),
        format!("WS_URL_LOCAL={url}\n"),
    )
    .expect("write endpoints");

    write_request(
        &root.join("requests/health.ws.json"),
        "ping",
        serde_json::json!({"jq": ".ok == true"}),
    );

    let out = run_api_websocket(
        root,
        &[
            "call",
            "--config-dir",
            "setup/websocket",
            "--env",
            "local",
            "requests/health.ws.json",
        ],
        &[],
    );

    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());
    assert_eq!(out.stdout_text(), "{\"ok\":true}");

    let history =
        std::fs::read_to_string(root.join("setup/websocket/.ws_history")).expect("history");
    assert!(history.contains("api-websocket call"));
    assert!(history.contains("--config-dir 'setup/websocket'"));
    assert!(history.contains("--env 'local'"));
    assert!(history.contains("requests/health.ws.json"));

    handle.join().expect("join websocket server");
}

#[test]
fn history_tail_command_only_omits_metadata_lines() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup = root.join("setup/websocket");
    std::fs::create_dir_all(&setup).expect("mkdir setup");

    std::fs::write(
        setup.join(".ws_history"),
        "# stamp exit=0 setup_dir=.\napi-websocket call \\\n  --config-dir 'setup/websocket' \\\n  requests/one.ws.json \\\n| jq .\n\n# stamp exit=0 setup_dir=.\napi-websocket call \\\n  --config-dir 'setup/websocket' \\\n  requests/two.ws.json \\\n| jq .\n\n",
    )
    .expect("write history");

    let out = run_api_websocket(
        root,
        &[
            "history",
            "--config-dir",
            "setup/websocket",
            "--tail",
            "1",
            "--command-only",
        ],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());
    assert!(out.stdout_text().contains("requests/two.ws.json"));
    assert!(!out.stdout_text().contains("stamp exit"));
}

#[test]
fn call_expect_failure_non_json_prints_response_preview_to_stderr() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/websocket")).expect("mkdir setup");
    std::fs::create_dir_all(root.join("requests")).expect("mkdir requests");

    let (url, handle) = spawn_echo_server();

    write_json(
        &root.join("requests/fail.ws.json"),
        &serde_json::json!({
            "steps": [
                {"type": "send", "text": "plain"},
                {"type": "receive"},
                {"type": "close"}
            ],
            "expect": {"jq": ".ok == true"}
        }),
    );

    let out = run_api_websocket(
        root,
        &[
            "call",
            "--config-dir",
            "setup/websocket",
            "--url",
            &url,
            "--no-history",
            "requests/fail.ws.json",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    let stderr = out.stderr_text();
    assert!(stderr.contains("jq requires a JSON response text"));
    assert!(stderr.contains("Response body (non-JSON; first 8192 bytes):"));
    assert!(stderr.contains("boom-body"));

    handle.join().expect("join websocket server");
}

#[test]
fn report_run_writes_markdown_report() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/websocket")).expect("mkdir setup");
    std::fs::create_dir_all(root.join("requests")).expect("mkdir requests");
    std::fs::create_dir_all(root.join("out")).expect("mkdir out");

    let (url, handle) = spawn_echo_server();

    write_json(
        &root.join("requests/health.ws.json"),
        &serde_json::json!({
            "steps": [
                {"type": "send", "text": "ping"},
                {"type": "receive"},
                {"type": "close"}
            ],
            "expect": {"jq": ".ok == true"}
        }),
    );

    let out = run_api_websocket(
        root,
        &[
            "report",
            "--case",
            "ws-health",
            "--request",
            "requests/health.ws.json",
            "--run",
            "--url",
            &url,
            "--config-dir",
            "setup/websocket",
            "--out",
            "out/ws-health.md",
        ],
        &[],
    );

    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());
    assert!(out.stdout_text().contains("out/ws-health.md"));

    let markdown = std::fs::read_to_string(root.join("out/ws-health.md")).expect("report file");
    assert!(markdown.contains("Test Case: ws-health"));
    assert!(markdown.contains("Result: PASS"));
    assert!(markdown.contains("api-websocket call"));
    assert!(markdown.contains("### Assertions"));
    assert!(markdown.contains("### WebSocket Request"));
    assert!(markdown.contains("### Transcript"));

    handle.join().expect("join websocket server");
}

#[test]
fn report_from_cmd_with_response_file_generates_report() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/websocket")).expect("mkdir setup");
    std::fs::create_dir_all(root.join("requests")).expect("mkdir requests");
    std::fs::create_dir_all(root.join("responses")).expect("mkdir responses");
    std::fs::create_dir_all(root.join("out")).expect("mkdir out");

    write_request(
        &root.join("requests/health.ws.json"),
        "ping",
        serde_json::json!({"textContains": "ok"}),
    );

    std::fs::write(
        root.join("responses/transcript.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "target": "ws://example/ws",
            "transcript": [
                {"direction": "send", "payload": "ping"},
                {"direction": "receive", "payload": "{\"ok\":true}"}
            ],
            "lastReceived": "{\"ok\":true}"
        }))
        .expect("serialize transcript"),
    )
    .expect("write response file");

    let snippet =
        "api-websocket call --config-dir setup/websocket --env local requests/health.ws.json";
    let out = run_api_websocket(
        root,
        &[
            "report-from-cmd",
            "--response",
            "responses/transcript.json",
            "--out",
            "out/from-cmd.md",
            snippet,
        ],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());

    let markdown = std::fs::read_to_string(root.join("out/from-cmd.md")).expect("report file");
    assert!(markdown.contains("Test Case: health"));
    assert!(markdown.contains("Result: (response provided; request not executed)"));
    assert!(markdown.contains("### WebSocket Request"));
}
