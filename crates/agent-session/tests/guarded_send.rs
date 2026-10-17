//! Admission regressions use a modeled tmux executable and synthetic records.
#[path = "guarded_send/live_composer.rs"]
mod live_composer;
#[path = "guarded_send/native_composer.rs"]
mod native_composer;
#[path = "guarded_send/repair.rs"]
mod repair;
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
        let temp = tempfile::tempdir_in("/tmp").expect("short fixture socket paths");
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
    if key == 'AGENT_SESSION_RUNTIME_ID':
        path = root / 'runtime-read-count'
        count = int(path.read_text()) + 1 if path.exists() else 1
        path.write_text(str(count))
        if (root / 'flip-second-runtime-read').exists() and count == 2:
            values[key] = 'runtime-replaced'
        if (root / 'runtime-replaced').exists(): values[key] = 'runtime-replaced'
    if key in values: print(key + '=' + values[key]); sys.exit(0)
    sys.exit(1)
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
    if (root / 'draft-on-load').exists(): (root / 'composer').write_text('new user draft')
    if (root / 'capture-fails-on-load').exists(): (root / 'capture-fails').touch()
    if (root / 'start-on-load').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
elif command == 'paste-buffer':
    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
    if (root / 'fail-capture-after-paste').exists(): (root / 'capture-fails').touch()
    if (root / 'inject-draft').exists():
        with (root / 'composer').open('a') as out: out.write(' user addition')
    if (root / 'start-after-paste').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
elif command == 'send-keys' and args[-1] == 'BSpace':
    if not (root / 'backspace-ignored').exists():
        (root / 'composer').write_text((root / 'composer').read_text()[:-1])
elif command == 'send-keys' and args[-1] == 'Enter':
    with (root / 'submissions').open('a') as out: out.write(json.dumps((root / 'composer').read_text()) + '\n')
    (root / 'composer').write_text('')
    if (root / 'replace-after-enter').exists(): (root / 'runtime-replaced').touch()
elif command == 'capture-pane':
    count_path = root / 'capture-count'
    count = int(count_path.read_text()) + 1 if count_path.exists() else 1
    count_path.write_text(str(count))
    if (root / 'fail-second-capture').exists() and count == 2: sys.exit(1)
    if (root / 'capture-fails').exists(): sys.exit(1)
    if (root / 'capture-oversize').exists():
        if provider == 'claude': print('────────────────────────────\n❯ \x1b[2mTry \"edit <filepath> to...\"\x1b[22m\n────────────────────────────')
        else: print('› \x1b[2mAsk Codex to do anything\x1b[22m\n')
        print('x' * (2 * 1024 * 1024))
        sys.exit(0)
    if (root / 'draft-fourth-capture').exists() and count >= 4: (root / 'composer').write_text('user addition')
    text = (root / 'composer').read_text()
    # Native capture trims occupied spaces without -N; -T excludes unused cells.
    occupied = text if '-N' in args else text.rstrip(' ')
    padding = ' ' * 8 if '-N' in args and '-T' not in args else ''
    native = (root / 'native-layout').exists()
    rows = ''
    footer = '\x1b]8;;https://example.invalid/\x1b\\status\x1b]8;;\x1b\\' if native else 'status'
    suggestion = (root / 'suggestion').read_text() if (root / 'suggestion').exists() else None
    top = '─── a long session title that fills the width of the pane ─' if (root / 'titled-border').exists() else '────────────────────────────'
    # Live Claude: a long title leaves no border dashes, and every composer row
    # is padded with written spaces that -N and -T keep.
    if (root / 'long-title').exists(): top = ' a deliberately long session title for the composer border lookup… ─'
    live = (root / 'live-layout').exists() and provider == 'claude'
    def pad(row): return row + ' ' * 20 if live else row
    if suggestion and not text:
        shown = ('\x1b[2m' + suggestion + '\x1b[22m') if '-e' in args else suggestion
        # Claude styles every suggestion word on its own; the spaces stay plain.
        if (root / 'suggestion-words').exists() and '-e' in args: shown = ' '.join('\x1b[2m' + w + '\x1b[0m' for w in suggestion.split(' '))
        if (root / 'suggestion-padding').exists(): shown += (root / 'suggestion-padding').read_text()
        if provider == 'claude': print(top + '\n❯\u00a0' + pad(shown) + '\n────────────────────────────\n  ' + footer)
        else: print('› ' + shown + '\n\n  ' + footer)
        sys.exit(0)
    if provider == 'claude':
        shown = occupied + padding if text or (root / 'bare-composer').exists() else ('\x1b[2mTry "edit <filepath> to..."\x1b[22m' if '-e' in args else 'Try "edit <filepath> to..."')
        print(top + '\n❯\u00a0' + pad(shown) + '\n' + rows + '────────────────────────────\n  ' + footer)
    else:
        shown = occupied + padding if text else ('\x1b[2mAsk Codex to do anything\x1b[22m' if '-e' in args else 'Ask Codex to do anything')
        print('› ' + shown + '\n' + rows + '\n  ' + footer)
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
        self.send_text(Some("continue-fixture"), extra)
    }

    fn send_text(&self, text: Option<&str>, extra: &[&str]) -> CmdOutput {
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
            "--key",
            "enter",
        ];
        if let Some(text) = text {
            args.extend_from_slice(&["--text", text]);
        }
        args.extend_from_slice(extra);
        run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .with_cwd(self.root())
                .with_env_remove_prefix("AGENT_SESSION_"),
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
        assert_eq!(value["error"]["code"], code, "{value}");
        value
    }
}

#[test]
fn guarded_send_draft_present_refuses_without_pasting() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), "user draft").unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        let value = fixture.refused(&output, "send-composer-not-empty");
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
        let value = fixture.refused(&output, "send-session-not-idle");
        assert_eq!(value["error"]["details"]["guard"], "idle");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_identity_and_key_happy_path_submits_once() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        let output = fixture.send(&[
            "--require-idle",
            "--require-empty-composer",
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
        assert_eq!(output.stdout_json()["data"]["guards"]["atomic"], false);
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

#[test]
fn guarded_send_post_paste_mismatch_preserves_composer_without_submit() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("inject-draft"), "").unwrap();
        let output = fixture.send(&[
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "mismatch",
        ]);
        fixture.refused(&output, "send-composer-mismatch");
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1
        );
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "continue-fixture user addition"
        );
        let repeated = fixture.send(&["--idempotency-key", "mismatch"]);
        fixture.refused(&repeated, "send-idempotency-key-used");
    }
}

#[test]
fn guarded_send_turn_start_after_paste_refuses_without_submit() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    fixture.refused(&output, "send-session-not-idle");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn guarded_send_unobservable_capture_and_activity_fail_closed() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("capture-fails"), "").unwrap();
    fixture.refused(
        &fixture.send(&["--require-empty-composer"]),
        "send-composer-unobservable",
    );
    fixture.no_delivery();
    fs::remove_file(fixture.root().join("capture-fails")).unwrap();
    fs::remove_file(fixture.root().join("state/sessions/guarded/activity.json")).unwrap();
    fixture.refused(&fixture.send(&["--require-idle"]), "send-idle-unobservable");
    fixture.no_delivery();
}

#[test]
fn guarded_send_unsupported_provider_checks_fail_closed() {
    let fixture = Fixture::new("dsh");
    for flag in ["--require-idle", "--require-empty-composer"] {
        fixture.refused(&fixture.send(&[flag]), "send-guard-unsupported");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_pending_provider_event_refuses_without_pasting() {
    use std::os::fd::AsRawFd;
    let fixture = Fixture::new("claude");
    let intent = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(
            fixture
                .root()
                .join("state/sessions/guarded/.activity-ingress.lock"),
        )
        .unwrap();
    // SAFETY: intent owns this valid descriptor; this is synthetic state only.
    assert_eq!(unsafe { libc::flock(intent.as_raw_fd(), libc::LOCK_SH) }, 0);
    let output = fixture.send(&["--require-idle"]);
    fixture.refused(&output, "send-provider-event-pending");
    fixture.no_delivery();
}

#[test]
fn guarded_send_concurrent_same_key_delivers_once() {
    let fixture = Fixture::new("claude");
    let mut codes = std::thread::scope(|scope| {
        let first = scope.spawn(|| fixture.send(&["--idempotency-key", "concurrent"]));
        let second = scope.spawn(|| fixture.send(&["--idempotency-key", "concurrent"]));
        let outputs = [first.join().unwrap(), second.join().unwrap()];
        for output in &outputs {
            if output.code != 0 {
                fixture.refused(output, "send-idempotency-key-used");
            }
        }
        outputs.map(|output| output.code)
    });
    codes.sort();
    assert_eq!(codes, [0, 1]);
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
fn guarded_send_ambiguous_composer_fails_closed() {
    for (provider, draft) in [
        ("codex", "Ask Codex to do anything"),
        ("claude", "[Pasted text #1 +2 lines]"),
        ("claude", "Try \"edit <filepath> to...\""),
    ] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), draft).unwrap();
        fixture.refused(
            &fixture.send(&["--require-empty-composer"]),
            "send-composer-unobservable",
        );
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_turn_start_during_buffer_load_refuses_without_pasting() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("start-on-load"), "").unwrap();
    fixture.refused(
        &fixture.send(&["--require-idle", "--idempotency-key", "load-race"]),
        "send-session-not-idle",
    );
    assert!(fixture.calls().iter().any(|call| call[0] == "load-buffer"));
    fixture.no_delivery();
}

#[test]
fn guarded_send_truncated_capture_is_unobservable() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("capture-oversize"), "").unwrap();
        fixture.refused(
            &fixture.send(&["--require-empty-composer"]),
            "send-composer-unobservable",
        );
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_observed_provider_hook_idle_is_supported() {
    let fixture = Fixture::new("claude");
    let path = fixture.root().join("state/sessions/guarded/activity.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["state"]["source"]["confidence"] = json!("observed");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
}

#[test]
fn guarded_send_inferred_or_console_idle_is_unobservable() {
    for (kind, confidence) in [
        ("provider_hook", "inferred"),
        ("console_observation", "observed"),
    ] {
        let fixture = Fixture::new("claude");
        let path = fixture.root().join("state/sessions/guarded/activity.json");
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["state"]["source"] = json!({"kind": kind, "confidence": confidence});
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        fixture.refused(&fixture.send(&["--require-idle"]), "send-idle-unobservable");
        fixture.no_delivery();
    }
}
