//! Offline model-picker behavior through the real CLI and a fake tmux/provider.
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

struct Picker {
    temp: tempfile::TempDir,
}

impl Picker {
    fn new(provider: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = root.join("state/sessions/picker");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session.json"), serde_json::to_vec(&json!({
            "schema_version": "agent-session.session.v1", "id": "picker", "agent": provider,
            "mode": "interactive", "cwd": root, "tmux_session": "fake-picker",
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {"kind": "tmux", "tmux_session": "fake-picker", "generation": 1,
                "launch_id": "picker-runtime", "started_at": "2030-01-01T00:00:00Z"},
            "provider_resume": {"provider": provider, "session_id": "same-conversation",
                "captured_at": "2030-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []}
        })).unwrap()).unwrap();
        fs::write(
            dir.join("activity.json"),
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.activity.v1", "runtime_id": "picker-runtime",
                "runtime_generation": 1, "seen_event_count": 0,
                "state": {"schema_version": "agent-session.turn-state.v1", "phase": "waiting",
                    "phase_changed_at": "2030-01-01T00:00:00Z", "revision": 2,
                    "source": {"kind": "provider_hook", "confidence": "authoritative"}}
            }))
            .unwrap(),
        )
        .unwrap();
        let settings = json!({"model": "initial-model", "effort": "medium"});
        for file in ["defaults.json", "live.json", "selection.json"] {
            fs::write(root.join(file), serde_json::to_vec(&settings).unwrap()).unwrap();
        }
        fs::write(root.join("calls"), "").unwrap();
        fs::write(
            root.join("tmux"),
            include_str!("fixtures/model-picker-tmux.py"),
        )
        .unwrap();
        fs::set_permissions(root.join("tmux"), fs::Permissions::from_mode(0o755)).unwrap();
        Self { temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn command(&self, command: &str, extra: &[&str]) -> CmdOutput {
        let state = self.root().join("state");
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            command,
            "picker",
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .with_cwd(self.root())
                .without_ambient_managed_session_env()
                .with_env(
                    "AGENT_SESSION_TMUX_BIN",
                    self.root().join("tmux").to_str().unwrap(),
                )
                .with_env("AGENT_SESSION_CODEX_BIN", "/nonexistent/provider")
                .with_env("AGENT_SESSION_CLAUDE_BIN", "/nonexistent/provider"),
        )
    }

    fn json(&self, file: &str) -> Value {
        serde_json::from_slice(&fs::read(self.root().join(file)).unwrap()).unwrap()
    }
}

#[test]
fn printable_picker_key_switches_each_provider_for_this_session_only() {
    for provider in ["codex", "claude"] {
        let picker = Picker::new(provider);
        let record_before = picker.json("state/sessions/picker/session.json");
        let defaults_before = fs::read(picker.root().join("defaults.json")).unwrap();
        let before = picker.command("glance", &[]);
        assert_eq!(before.code, 0, "{}", before.stderr_text());
        assert!(before.stdout_text().contains("initial-model"));
        let output = picker.command("send", &["--key", "down", "--key", "right", "--key", "s"]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["keys"],
            json!(["down", "right", "s"])
        );
        assert_eq!(
            picker.json("live.json"),
            json!({"model": "selected-model", "effort": "high"})
        );
        assert_eq!(
            fs::read(picker.root().join("defaults.json")).unwrap(),
            defaults_before
        );
        let after = picker.command("glance", &[]);
        assert_eq!(after.code, 0, "{}", after.stderr_text());
        assert!(after.stdout_text().contains("selected-model"));
        assert!(after.stdout_text().contains("high"));
        let record_after = picker.json("state/sessions/picker/session.json");
        assert_eq!(record_after["runtime"], record_before["runtime"]);
        assert_eq!(
            record_after["provider_resume"],
            record_before["provider_resume"]
        );
        let calls = fs::read_to_string(picker.root().join("calls")).unwrap();
        let calls: Vec<Value> = calls
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(calls.iter().any(|call| call == &json!(["send-keys", "-t", "fake-picker:0.0", "-l", "--", "s"])));
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call[0].as_str(), Some("load-buffer" | "paste-buffer")))
        );
    }
}

#[test]
fn printable_picker_keys_refuse_busy_unknown_and_pending_activity_without_input() {
    for provider in ["codex", "claude"] {
        for phase in ["working", "unknown", "needs_input", "missing", "pending"] {
            let picker = Picker::new(provider);
            let path = picker.root().join("state/sessions/picker/activity.json");
            let mut activity = picker.json("state/sessions/picker/activity.json");
            let _pending = if phase == "pending" {
                use std::os::fd::AsRawFd;
                let file = fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(path.parent().unwrap().join(".activity-ingress.lock"))
                    .unwrap();
                // SAFETY: the fixture owns this descriptor and synthetic state.
                assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) }, 0);
                Some(file)
            } else {
                None
            };
            if phase == "missing" {
                fs::remove_file(&path).unwrap();
            } else if phase != "pending" {
                activity["state"]["phase"] = json!(phase);
                fs::write(path, serde_json::to_vec(&activity).unwrap()).unwrap();
            }
            let before = picker.json("live.json");
            let output = picker.command("send", &["--key", "s", "--allow-blocked"]);
            assert_ne!(output.code, 0, "{provider} {phase}");
            assert_eq!(output.stdout_json()["ok"], false);
            assert_ne!(output.stdout_json()["error"]["code"], "parse-error");
            assert_eq!(picker.json("live.json"), before);
            let calls = fs::read_to_string(picker.root().join("calls")).unwrap();
            assert!(!calls.contains("send-keys"), "{provider} {phase}: {calls}");
        }
    }
}

#[test]
fn printable_key_is_one_character_and_cannot_inject_tmux_commands() {
    let picker = Picker::new("claude");
    for key in [";", "-", "é", " "] {
        let arg = format!("--key={key}");
        let output = picker.command("send", &[&arg]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(output.stdout_json()["data"]["keys"], json!([key]));
    }
    for key in ["", "ss", "C-z", "\n", "\x1b", "\t"] {
        let arg = format!("--key={key}");
        let output = picker.command("send", &[&arg]);
        assert_eq!(output.code, 64, "{key:?}: {}", output.stdout_text());
    }
    let calls = fs::read_to_string(picker.root().join("calls")).unwrap();
    for key in [";", "-", "é", " "] {
        assert!(
            calls
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .any(|call| call
                    == json!([
                        "send-keys",
                        "-t",
                        "fake-picker:0.0",
                        "-l",
                        "--",
                        if key == ";" { "\\;" } else { key }
                    ]))
        );
    }
}

#[test]
fn named_enter_remains_an_explicit_default_saving_action() {
    let picker = Picker::new("codex");
    let output = picker.command(
        "send",
        &["--require-idle", "--key", "down", "--key", "enter"],
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(picker.json("defaults.json")["model"], "selected-model");
    assert_eq!(picker.json("live.json")["model"], "selected-model");
}

#[test]
fn printable_picker_key_refuses_an_unsupported_provider_without_input() {
    let picker = Picker::new("dsh");
    let output = picker.command("send", &["--key", "s"]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "send-guard-unsupported"
    );
    assert!(
        !fs::read_to_string(picker.root().join("calls"))
            .unwrap()
            .contains("send-keys")
    );
}
