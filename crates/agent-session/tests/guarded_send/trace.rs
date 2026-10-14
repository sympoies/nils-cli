//! Privacy and observation regressions for opt-in native timing traces.
use super::{GUARDS, Server};
use crate::Fixture;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

const CONTROL: &str = "AGENT_SESSION_GUARDED_TRACE_FILE";
const PROMPT: &str = "private-fixture-input-unique-marker";

fn send(fixture: &Fixture, path: Option<&Path>) -> CmdOutput {
    send_input(fixture, path, Some(PROMPT), None)
}

fn send_input(
    fixture: &Fixture,
    path: Option<&Path>,
    text: Option<&str>,
    stdin: Option<&str>,
) -> CmdOutput {
    let state = fixture.root().join("state");
    let tmux = fixture.root().join("tmux");
    let mut args = vec![
        "--state-dir",
        state.to_str().unwrap(),
        "send",
        "guarded",
        "--tmux-bin",
        tmux.to_str().unwrap(),
        "--format",
        "json",
        "--key",
        "enter",
    ];
    if let Some(text) = text {
        args.extend_from_slice(&["--text", text]);
    }
    if stdin.is_some() {
        args.push("--text-stdin");
    }
    args.extend_from_slice(GUARDS);
    let mut options = CmdOptions::new()
        .with_cwd(fixture.root())
        .with_env_remove_prefix("AGENT_SESSION_");
    if let Some(stdin) = stdin {
        options = options.with_stdin_str(stdin);
    }
    if let Some(path) = path {
        options = options.with_env(CONTROL, path.to_str().unwrap());
    }
    run_resolved("agent-session", &args, &options)
}

fn trace_path(fixture: &Fixture) -> std::path::PathBuf {
    let dir = fixture.root().join("private-trace");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    dir.canonicalize().unwrap().join("trace.jsonl")
}

fn assert_receipt(output: &CmdOutput) {
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let value = output.stdout_json();
    assert_eq!(value["schema_version"], "cli.agent-session.send.v1");
    assert_eq!(value["data"]["submission"]["outcome"], "submitted");
    assert_eq!(value["data"]["submission"]["enter_presses"], 0);
    assert_eq!(value["data"]["native"]["confirmation"], "input_observed");
    assert_eq!(value["data"]["native"]["transport"], "codex_app_server");
    assert_eq!(value["data"]["guards"]["atomic"], false);
    assert_eq!(value["data"]["keys"], json!(["enter"]));
}

fn events(path: &Path) -> Vec<Value> {
    let bytes = fs::read_to_string(path).expect("opt-in native trace must exist");
    for forbidden in [
        PROMPT,
        "provider-one",
        "runtime-one",
        "turn-one",
        "input-one",
        "other input",
        "params",
        "result",
        "history",
        "credentials",
        "private-error-marker",
        "private-token-marker",
    ] {
        assert!(
            !bytes.contains(forbidden),
            "raw field persisted: {forbidden}"
        );
    }
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let rows: Vec<Value> = bytes
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!rows.is_empty());
    assert!(rows.len() <= 256);
    let mut previous = 0;
    for row in &rows {
        assert_eq!(row["schema_version"], "agent-session.guarded-trace.v1");
        let elapsed = row["elapsed_us"].as_u64().unwrap();
        assert!(elapsed >= previous);
        previous = elapsed;
    }
    rows
}

#[test]
fn native_trace_opt_in_correlates_ack_and_reads_without_persisting_input() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "history-lag");
    let path = trace_path(&fixture);
    let output = send(&fixture, Some(&path));
    assert_receipt(&output);
    let rows = events(&path);
    assert!(rows.iter().any(|r| r["stage"] == "record_lock_acquired"));
    assert!(rows.iter().any(|r| r["stage"] == "rpc_response"
        && r["method"] == "turn_start"
        && r["outcome"] == "ok"));
    assert!(
        rows.iter()
            .any(|r| r["stage"] == "input_observation" && r["outcome"] == "not_observed")
    );
    assert!(
        rows.iter()
            .any(|r| r["stage"] == "input_observation" && r["outcome"] == "observed")
    );
    assert!(
        rows.iter()
            .any(|r| r["stage"] == "turn_acknowledged" && r["outcome"] == "ok")
    );
    for request in rows.iter().filter(|r| r["stage"] == "rpc_request") {
        assert!(rows.iter().any(|r| r["stage"] == "rpc_response"
            && r["request"] == request["request"]
            && r["method"] == request["method"]));
    }
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}

#[test]
fn native_trace_confirmation_failure_preserves_key_and_refusal() {
    for mode in [
        "wrong-input",
        "history-timeout",
        "disconnect",
        "rpc-error",
        "invalid-ack",
    ] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, mode);
        let path = trace_path(&fixture);
        let output = send(&fixture, Some(&path));
        let value = fixture.refused(&output, "send-native-unconfirmed");
        let acknowledged = matches!(mode, "wrong-input" | "history-timeout");
        assert_eq!(value["error"]["details"]["acknowledged"], acknowledged);
        assert_eq!(
            value["error"]["details"]["submitted"],
            if acknowledged {
                json!(true)
            } else {
                Value::Null
            }
        );
        assert_eq!(value["error"]["details"]["pasted"], false);
        assert_eq!(value["error"]["details"]["dispatched"], true);
        let rows = events(&path);
        let (stage, method, outcome) = match mode {
            "wrong-input" => ("input_observation", None, "invalid"),
            "history-timeout" => ("rpc_response", Some("thread_read"), "timeout"),
            "rpc-error" => ("rpc_response", Some("turn_start"), "rejected"),
            "invalid-ack" => ("turn_acknowledged", None, "invalid"),
            _ => ("rpc_response", Some("turn_start"), "unavailable"),
        };
        assert!(
            rows.iter().any(|r| r["stage"] == stage
                && r["method"].as_str() == method
                && r["outcome"] == outcome
                && (mode != "history-timeout" || r["include_turns"] == true)),
            "{rows:?}"
        );
        fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
        assert_eq!(server.mutations().len(), 1);
        fixture.no_delivery();
    }
}

#[test]
fn native_trace_disabled_creates_no_artifact() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    assert_receipt(&send(&fixture, None));
    assert!(!fixture.root().join("trace.jsonl").exists());
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}

#[test]
fn native_trace_unsafe_paths_do_not_change_dispatch_or_overwrite_files() {
    for mode in ["existing", "symlink", "public-parent", "relative"] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, "idle");
        fs::set_permissions(fixture.root(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = fixture.root().canonicalize().unwrap();
        let saved = root.join("saved");
        fs::write(&saved, "fixture-original").unwrap();
        let path = match mode {
            "existing" => saved.clone(),
            "symlink" => {
                let p = root.join("link");
                symlink(&saved, &p).unwrap();
                p
            }
            "public-parent" => {
                let p = root.join("public");
                fs::create_dir(&p).unwrap();
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
                p.join("trace")
            }
            _ => std::path::PathBuf::from("relative-trace"),
        };
        assert_receipt(&send(&fixture, Some(&path)));
        assert_eq!(fs::read_to_string(&saved).unwrap(), "fixture-original");
        if matches!(mode, "public-parent" | "relative") {
            assert!(!fixture.root().join(path).exists());
        }
        assert_eq!(server.mutations().len(), 1);
        fixture.no_delivery();
    }
}

#[test]
fn native_trace_draft_or_busy_admission_refuses_without_input() {
    for mode in ["draft", "busy"] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, if mode == "busy" { "busy" } else { "idle" });
        if mode == "draft" {
            fs::write(fixture.root().join("composer"), "private-draft-marker").unwrap();
        }
        let path = trace_path(&fixture);
        let output = send(&fixture, Some(&path));
        fixture.refused(
            &output,
            if mode == "busy" {
                "send-session-not-idle"
            } else {
                "send-composer-not-empty"
            },
        );
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("private-draft-marker"));
        events(&path);
        assert!(server.mutations().is_empty());
        fixture.no_delivery();
    }
}

#[test]
fn native_trace_initialize_timeout_is_classified_without_a_mutation() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "initialize-timeout");
    let path = trace_path(&fixture);
    fixture.refused(&send(&fixture, Some(&path)), "send-idle-unobservable");
    let rows = events(&path);
    assert!(
        rows.iter().any(|r| r["stage"] == "rpc_response"
            && r["method"] == "connect"
            && r["outcome"] == "timeout"),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|r| r["stage"] == "rpc_response"
            && r["method"] == "initialize"
            && matches!(r["outcome"].as_str(), Some("timeout" | "cancelled"))),
        "{rows:?}"
    );
    assert!(server.mutations().is_empty());
    fixture.no_delivery();
}

#[test]
fn native_trace_large_history_preserves_enabled_and_disabled_receipts() {
    let mut receipts = Vec::new();
    for enabled in [false, true] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, "large-history");
        let path = trace_path(&fixture);
        let output = send(&fixture, enabled.then_some(path.as_path()));
        assert_receipt(&output);
        receipts.push(output.stdout_json());
        if enabled {
            let rows = events(&path);
            assert!(rows.iter().any(|r| r["stage"] == "input_observation"
                && r["outcome"] == "observed"
                && r["thread_matches"] == true
                && r["turn_matches"] == true
                && r["text_matches"] == true));
        } else {
            assert!(!path.exists());
        }
        assert_eq!(server.mutations().len(), 1);
        fixture.no_delivery();
    }
    assert_eq!(receipts[0], receipts[1]);
}

fn assert_terminal_trace_absent(text: Option<&str>, stdin: Option<&str>, enters: usize) {
    let mut receipts = Vec::new();
    for enabled in [false, true] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, "idle");
        let path = trace_path(&fixture);
        let output = send_input(&fixture, enabled.then_some(path.as_path()), text, stdin);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let receipt = output.stdout_json();
        assert!(receipt["data"].get("native").is_none());
        assert!(receipt["data"].get("submission").is_none());
        assert_eq!(receipt["data"]["guards"]["atomic"], false);
        assert_eq!(receipt["data"]["keys"], json!(["enter"]));
        assert_eq!(
            receipt["data"]["sent_text"],
            text.is_some() || stdin.is_some()
        );
        receipts.push(receipt);
        assert!(server.mutations().is_empty());
        let calls = fixture.calls();
        assert!(calls.iter().all(|call| call[0] != "paste-buffer"));
        assert_eq!(
            calls.iter().filter(|call| call[0] == "send-keys").count(),
            enters
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions")).unwrap(),
            "\"\"\n".repeat(enters)
        );
        assert!(
            !path.exists(),
            "terminal delivery must not create a native trace"
        );
    }
    assert_eq!(receipts[0], receipts[1]);
}

#[test]
fn native_trace_key_only_terminal_delivery_creates_no_artifact() {
    assert_terminal_trace_absent(None, None, 1);
}

#[test]
fn native_trace_bare_newline_terminal_delivery_creates_no_artifact() {
    // A bare newline and the explicit Enter each retain their terminal key.
    assert_terminal_trace_absent(Some("\n"), None, 2);
    assert_terminal_trace_absent(None, Some("\n"), 2);
}

#[test]
fn native_trace_stdin_literal_input_uses_native_route() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let path = trace_path(&fixture);
    assert_receipt(&send_input(&fixture, Some(&path), None, Some(PROMPT)));
    events(&path);
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}
