//! Synthetic Claude Code 2.1.317 hooks; no provider process or live pane.
use super::*;
use pretty_assertions::assert_eq;

fn hook(f: &Fixture, name: &str, call: Option<&str>) {
    hook_tool(f, name, call, "Bash");
}
fn hook_tool(f: &Fixture, name: &str, call: Option<&str>, tool: &str) {
    let mut payload = json!({
        "hook_event_name": name, "session_id": "provider-one", "prompt_id": "turn-one",
        "tool_name": tool, "tool_input": {"command": "fixture-operation"}
    });
    if let Some(call) = call {
        payload["tool_use_id"] = json!(call);
    }
    let out = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            f.root().join("state").to_str().unwrap(),
            "activity",
            "hook",
            "--agent",
            "claude",
        ],
        &CmdOptions::new()
            .with_cwd(f.root())
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("AGENT_SESSION_ID", "guarded")
            .with_env("AGENT_SESSION_RUNTIME_ID", "runtime-one")
            .with_stdin_str(&payload.to_string()),
    );
    assert_eq!(out.code, 0, "{}", out.stderr_text());
}

#[test]
fn claude_permission_keystroke_cannot_be_bound_to_the_pending_call() {
    let f = Fixture::new("claude");
    hook(&f, "UserPromptSubmit", None);
    hook(&f, "PreToolUse", Some("call-one"));
    hook(&f, "PermissionRequest", None);
    let state: Value = serde_json::from_slice(
        &fs::read(f.root().join("state/sessions/guarded/activity.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["state"]["phase"], "needs_input");
    // The modeled pane is an empty composer, even though exact hook attention
    // is pending. Enter has no request id and can land in that composer.
    let out = f.send_text(None, &[]);
    f.refused(&out, "claude-permission-hook-required");
    f.refused(
        &f.send_text(Some("yes"), &["--allow-blocked"]),
        "claude-permission-hook-required",
    );
    f.refused(
        &f.send_text(Some("\n"), &[]),
        "claude-permission-hook-required",
    );
    f.no_delivery();
}

#[test]
fn claude_permission_hook_answers_and_refuses_mismatches() {
    for decision in ["allow", "deny"] {
        let f = Fixture::new("claude");
        hook(&f, "UserPromptSubmit", None);
        hook(&f, "PreToolUse", Some("call-one"));
        hook(&f, "PermissionRequest", None);
        let state: Value = serde_json::from_slice(
            &fs::read(f.root().join("state/sessions/guarded/activity.json")).unwrap(),
        )
        .unwrap();
        let attention = state["pending_attention"][0]["id"].as_str().unwrap();
        let provider = f.root().join("claude-fixture");
        fs::write(&provider, "#!/bin/sh\nprintf '2.1.317 (Claude Code)\\n'\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_agent-session"))
            .args([
                "--state-dir",
                f.root().join("state").to_str().unwrap(),
                "permission",
                "wait",
                "guarded",
                "--runtime-id",
                "runtime-one",
                "--attention-id",
                attention,
            ])
            .env_remove("AGENT_SESSION_ID")
            .env_remove("AGENT_SESSION_RUNTIME_ID")
            .env("AGENT_SESSION_CLAUDE_BIN", &provider)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut pending = json!([]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            let list = permission(&f, &["list", "guarded", "--format", "json"]);
            if list.code == 0 {
                pending = list.stdout_json()["data"]["requests"].clone();
                if pending.as_array().is_some_and(|a| !a.is_empty()) {
                    break;
                }
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let requests = pending.as_array().expect("request array");
        if requests.is_empty() {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "live hook did not publish its correlated request: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let request = &requests[0];
        let id = request["request_id"].as_str().unwrap();
        let call = request["call_id"].as_str().unwrap();
        let stale = permission(
            &f,
            &[
                "answer",
                "guarded",
                "--request-id",
                "00000000-0000-4000-8000-000000000000",
                "--call-id",
                call,
                "--decision",
                decision,
                "--format",
                "json",
            ],
        );
        f.refused(&stale, "permission-request-gone");
        let wrong = permission(
            &f,
            &[
                "answer",
                "guarded",
                "--request-id",
                id,
                "--call-id",
                "other-call",
                "--decision",
                decision,
                "--format",
                "json",
            ],
        );
        f.refused(&wrong, "permission-call-mismatch");
        assert!(
            child.try_wait().unwrap().is_none(),
            "mismatch consumed the hook"
        );
        let answer = [
            "answer",
            "guarded",
            "--request-id",
            id,
            "--call-id",
            call,
            "--decision",
            decision,
            "--format",
            "json",
        ];
        let result = permission(&f, &answer);
        assert_eq!(result.code, 0, "{}", result.stdout_text());
        assert_eq!(result.stdout_json()["data"]["outcome"], "delivered_to_hook");
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value,
            json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": decision}}})
        );
        f.refused(&permission(&f, &answer), "permission-request-gone");
        f.no_delivery();
    }
}

fn permission(f: &Fixture, args: &[&str]) -> CmdOutput {
    let state = f.root().join("state");
    let mut all = vec!["--state-dir", state.to_str().unwrap(), "permission"];
    all.extend_from_slice(args);
    run_resolved(
        "agent-session",
        &all,
        &CmdOptions::new()
            .with_cwd(f.root())
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env(
                "AGENT_SESSION_CLAUDE_BIN",
                f.root().join("claude-fixture").to_str().unwrap(),
            ),
    )
}

fn fixture_version(f: &Fixture, version: &str) -> std::path::PathBuf {
    let provider = f.root().join("claude-fixture");
    fs::write(
        &provider,
        format!("#!/bin/sh\nprintf '%s\\n' '{version} (Claude Code)'\n"),
    )
    .unwrap();
    fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
    provider
}
fn start_wait(f: &Fixture, attention: &str) -> std::process::Child {
    std::process::Command::new(env!("CARGO_BIN_EXE_agent-session"))
        .args([
            "--state-dir",
            f.root().join("state").to_str().unwrap(),
            "permission",
            "wait",
            "guarded",
            "--runtime-id",
            "runtime-one",
            "--attention-id",
            attention,
        ])
        .env("AGENT_SESSION_CLAUDE_BIN", f.root().join("claude-fixture"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
fn attention_id(f: &Fixture) -> String {
    let state: Value = serde_json::from_slice(
        &fs::read(f.root().join("state/sessions/guarded/activity.json")).unwrap(),
    )
    .unwrap();
    state["pending_attention"][0]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn claude_permission_missing_or_ambiguous_call_sends_no_decision() {
    for calls in [0, 2] {
        let f = Fixture::new("claude");
        fixture_version(&f, "2.1.317");
        hook(&f, "UserPromptSubmit", None);
        if calls == 2 {
            hook(&f, "PreToolUse", Some("call-a"));
            hook(&f, "PreToolUse", Some("call-b"));
        }
        hook(&f, "PermissionRequest", None);
        let output = start_wait(&f, &attention_id(&f))
            .wait_with_output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("permission-request-uncorrelated")
        );
        assert!(permission(&f, &["list", "guarded", "--format", "json"]).stdout_json()["data"]["requests"].as_array().unwrap().is_empty());
        f.no_delivery();
    }
}

#[test]
fn claude_permission_version_gate_refuses_unreviewed_and_unknown_versions() {
    for version in ["2.1.316", "2.1.318", "3.0.0", "unknown"] {
        let f = Fixture::new("claude");
        fixture_version(&f, version);
        hook(&f, "UserPromptSubmit", None);
        hook(&f, "PreToolUse", Some("call-one"));
        hook(&f, "PermissionRequest", None);
        let output = start_wait(&f, &attention_id(&f))
            .wait_with_output()
            .unwrap();
        let code = if version == "unknown" {
            "permission-provider-version-unavailable"
        } else {
            "permission-provider-version-unsupported"
        };
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(code));
        f.refused(
            &permission(&f, &["list", "guarded", "--format", "json"]),
            code,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(
                &fs::read(f.root().join("state/sessions/guarded/activity.json")).unwrap()
            )
            .unwrap()["state"]["phase"],
            "needs_input"
        );
        f.no_delivery();
    }
}

#[test]
fn claude_permission_prompt_resolved_during_answer_sends_no_decision() {
    use std::io::{Read, Write};
    let f = Fixture::new("claude");
    fixture_version(&f, "2.1.317");
    hook(&f, "UserPromptSubmit", None);
    hook(&f, "PreToolUse", Some("call-one"));
    hook(&f, "PermissionRequest", None);
    let mut child = start_wait(&f, &attention_id(&f));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let request = loop {
        let requests = permission(&f, &["list", "guarded", "--format", "json"]).stdout_json()["data"]["requests"].clone();
        if let Some(request) = requests.as_array().and_then(|a| a.first()) {
            break request.clone();
        }
        if child.try_wait().unwrap().is_some() || std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hook did not register");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    // A second process cannot register a second answerer for the same call.
    let duplicate = start_wait(&f, &attention_id(&f))
        .wait_with_output()
        .unwrap();
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("permission-hook-already-waiting"));
    let path = f
        .root()
        .join("state/sessions/guarded/permissions")
        .join(format!("{}.json", request["request_id"].as_str().unwrap()));
    let private: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut stream =
        std::os::unix::net::UnixStream::connect(private["socket"].as_str().unwrap()).unwrap();
    // Resolving this prompt invalidates its socket and UUID. Final emission
    // after a complete answer is covered deterministically by transport tests;
    // connect alone does not prove the waiter has accepted this socket.
    hook(&f, "PostToolUse", Some("call-one"));
    let bytes = json!({"request_id": request["request_id"], "call_id": request["call_id"], "decision":"allow"}).to_string();
    let _ = stream.write_all(bytes.as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stale answer emitted a permission decision"
    );
    assert!(reply.is_empty());
    let stale = permission(
        &f,
        &[
            "answer",
            "guarded",
            "--request-id",
            request["request_id"].as_str().unwrap(),
            "--call-id",
            request["call_id"].as_str().unwrap(),
            "--decision",
            "allow",
            "--format",
            "json",
        ],
    );
    f.refused(&stale, "permission-request-gone");
    f.no_delivery();
}

#[test]
fn claude_permission_after_clarification_still_refuses_all_terminal_answers() {
    let f = Fixture::new("claude");
    hook(&f, "UserPromptSubmit", None);
    hook_tool(&f, "PreToolUse", Some("question-call"), "AskUserQuestion");
    hook(&f, "PreToolUse", Some("call-one"));
    hook(&f, "PermissionRequest", None);
    let state: Value = serde_json::from_slice(
        &fs::read(f.root().join("state/sessions/guarded/activity.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["state"]["current_turn"]["attention"]["kind"],
        "clarification"
    );
    assert!(
        state["pending_attention"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "approval")
    );
    f.refused(&f.send_text(None, &[]), "claude-permission-hook-required");
    f.refused(
        &f.send_text(Some("yes"), &["--allow-blocked"]),
        "claude-permission-hook-required",
    );
    f.refused(
        &f.send_text(Some("\n"), &[]),
        "claude-permission-hook-required",
    );
    assert!(f.calls().is_empty());
}

#[test]
fn claude_permission_pending_ingress_refuses_terminal_answers() {
    use std::os::fd::AsRawFd;
    let f = Fixture::new("claude");
    let ingress = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(
            f.root()
                .join("state/sessions/guarded/.activity-ingress.lock"),
        )
        .unwrap();
    // Model a provider hook advertising ingress before waiting for the send's
    // record lock. No provider event can commit while the caller owns it.
    assert_eq!(
        unsafe { libc::flock(ingress.as_raw_fd(), libc::LOCK_SH) },
        0
    );
    f.refused(&f.send_text(None, &[]), "send-provider-event-pending");
    assert!(f.calls().is_empty());
}

#[test]
fn claude_permission_ingress_cannot_overtake_a_guarded_paste() {
    let f = Fixture::new("claude");
    let model = f.root().join("tmux-model");
    let script = fs::read_to_string(&model).unwrap().replace(
        "elif command == 'paste-buffer':",
        r#"elif command == 'paste-buffer':
    import fcntl
    with (root / 'state/sessions/guarded/.activity-ingress.lock').open('a+') as ingress:
        try:
            fcntl.flock(ingress, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            pass
        else:
            (root / 'ingress-overtook-paste').touch()
"#,
    );
    fs::write(model, script).unwrap();
    let output = f.send(&["--require-idle", "--require-empty-composer"]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert!(
        !f.root().join("ingress-overtook-paste").exists(),
        "provider ingress overtook the guarded paste's final admission"
    );
}
