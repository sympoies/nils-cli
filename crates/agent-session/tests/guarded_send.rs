//! Admission regressions use a modeled tmux executable and synthetic records.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new(provider: &str) -> Self {
        let temp = tempfile::TempDir::new().expect("fixture");
        let dir = temp.path().join("state/sessions/guarded");
        fs::create_dir_all(&dir).unwrap();
        let record = json!({
            "schema_version": "agent-session.session.v1", "id": "guarded",
            "agent": provider, "mode": "interactive", "title": null,
            "cwd": temp.path(), "tmux_session": "modeled-guarded",
            "prompt_file": null, "log_file": null,
            "created_at": "2000-01-01T00:00:00Z", "updated_at": "2000-01-01T00:00:00Z",
            "runtime": { "kind": "tmux", "tmux_session": "modeled-guarded",
                "generation": 1, "launch_id": "runtime-one", "started_at": "2000-01-01T00:00:00Z" },
            "provider_resume": { "provider": provider, "session_id": "provider-one",
                "captured_at": "2000-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": [] }
        });
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        fs::write(temp.path().join("provider"), provider).unwrap();
        fs::write(temp.path().join("composer"), "").unwrap();
        fs::write(temp.path().join("calls"), "").unwrap();
        let script = r#"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'calls').open('a') as out: out.write(json.dumps(args) + '\n')
command = args[0]
provider = (root / 'provider').read_text()
if command == 'has-session':
    if (root / 'start-turn').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
    sys.exit(0)
if command == 'show-environment':
    key = args[-1]
    values = {'AGENT_SESSION_ID': 'guarded', 'AGENT_SESSION_RUNTIME_ID': 'runtime-one', 'AGENT_SESSION_STATE_DIR': str(root / 'state')}
    if key in values: print(key + '=' + values[key]); sys.exit(0)
    sys.exit(1)
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
elif command == 'paste-buffer':
    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
elif command == 'send-keys' and args[-1] == 'Enter':
    with (root / 'submissions').open('a') as out: out.write(json.dumps((root / 'composer').read_text()) + '\n')
    (root / 'composer').write_text('')
elif command == 'capture-pane':
    text = (root / 'composer').read_text()
    if provider == 'claude': print('────────────────────────────\n❯ ' + text + '\n────────────────────────────\n  status')
    else: print('› ' + (text or 'Ask Codex to do anything') + '\n\n  status')
"#;
        let tmux = temp.path().join("tmux");
        fs::write(&tmux, script).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        let fixture = Self { temp };
        fixture.idle();
        fixture
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn idle(&self) {
        let document = json!({
            "schema_version": "agent-session.activity.v1", "runtime_id": "runtime-one",
            "runtime_generation": 1, "seen_event_count": 0,
            "state": { "schema_version": "agent-session.turn-state.v1", "phase": "waiting",
                "phase_changed_at": "2000-01-01T00:00:00Z", "revision": 2,
                "source": { "kind": "provider_hook", "confidence": "authoritative" } }
        });
        fs::write(
            self.root().join("state/sessions/guarded/activity.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
    }

    fn send(&self, extra: &[&str]) -> CmdOutput {
        let state = self.root().join("state");
        let tmux = self.root().join("tmux");
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "send",
            "guarded",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
            "--text",
            "continue-fixture",
            "--key",
            "enter",
        ];
        args.extend_from_slice(extra);
        run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new().with_cwd(self.root()),
        )
    }

    fn calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.root().join("calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn no_delivery(&self) {
        assert!(
            self.calls()
                .iter()
                .all(|call| !matches!(call[0].as_str(), "paste-buffer" | "send-keys"))
        );
        assert!(!self.root().join("submissions").exists());
    }

    fn refused(&self, output: &CmdOutput, code: &str) -> Value {
        assert_eq!(output.code, 1, "{}", output.stderr_text());
        let value = output.stdout_json();
        assert_eq!(value["error"]["code"], code);
        value
    }
}

#[test]
fn guarded_send_draft_present_refuses_without_pasting() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), "user draft").unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        let value = fixture.refused(&output, "send-guard-unsupported");
        assert_eq!(value["error"]["details"]["guard"], "empty_composer");
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "user draft"
        );
    }
}

#[test]
fn guarded_send_turn_started_after_caller_check_refuses_without_pasting() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        // A caller observed waiting. The modeled liveness probe now starts a
        // turn before send's final admission; pane adapters cannot fence it.
        fs::write(fixture.root().join("start-turn"), "").unwrap();
        let output = fixture.send(&["--require-idle"]);
        let value = fixture.refused(&output, "send-guard-unsupported");
        assert_eq!(value["error"]["details"]["guard"], "idle");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_identity_and_key_happy_path_submits_once() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        let output = fixture.send(&[
            "--expect-runtime",
            "runtime-one",
            "--expect-provider-session",
            "provider-one",
            "--idempotency-key",
            "continuation-one",
        ]);
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["outcome"],
            "submitted"
        );
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[test]
fn guarded_send_repeated_key_survives_process_restart_and_runtime_change() {
    let fixture = Fixture::new("claude");
    let first = fixture.send(&["--idempotency-key", "continuation-one"]);
    assert_eq!(first.code, 0, "{}", first.stderr_text());
    let path = fixture.root().join("state/sessions/guarded/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("runtime-two");
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let repeated = fixture.send(&["--idempotency-key", "continuation-one"]);
    fixture.refused(&repeated, "send-idempotency-key-used");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn guarded_send_identity_mismatch_refuses_without_delivery() {
    for guard in ["--expect-runtime", "--expect-provider-session"] {
        let fixture = Fixture::new("claude");
        let output = fixture.send(&[guard, "replaced-identity"]);
        fixture.refused(&output, "send-identity-mismatch");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_unguarded_output_remains_compatible() {
    let fixture = Fixture::new("codex");
    let output = fixture.send(&[]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let value = output.stdout_json();
    assert_eq!(value["schema_version"], "cli.agent-session.send.v1");
    assert_eq!(value["data"]["sent_text"], true);
    assert!(value["data"].get("guards").is_none());
    assert!(value["data"].get("idempotency_key").is_none());
}
