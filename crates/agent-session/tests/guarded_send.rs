//! Admission regressions use a modeled tmux executable and synthetic records.
#[path = "guarded_send/exit.rs"]
mod exit;
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

const TMUX_FAST_PATH: &str = include_str!("guarded_send/tmux-fast-path.sh");

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
            "delete_tmux_identity": { "launch_id": "runtime-one", "session_id": "$42",
                "pane_id": "%42", "pane_pid": 42 },
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
import atexit, json, pathlib, sys, textwrap
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'calls').open('a') as out: out.write(json.dumps(args) + '\n')
command = args[0]
provider = (root / 'provider').read_text()
def print_cursor():
    text = (root / 'composer').read_text()
    cursor = int((root / 'cursor').read_text()) if (root / 'cursor').exists() else len(text)
    x, y = 2 + cursor, 1
    if (root / 'wrap-80').exists() and len(text) > 78:
        chunks = [text[i:i + 78] for i in range(0, len(text), 78)]
        if (root / 'word-wrap').exists(): chunks = textwrap.wrap(text, width=78, drop_whitespace=False)
        for row, chunk in enumerate(chunks):
            if cursor <= len(chunk): x, y = 2 + cursor, 1 + row; break
            cursor -= len(chunk)
    if (root / 'captured-layout').exists():
        snapshot = json.loads((root / 'captured-layout').read_text())
        if text == snapshot['input']:
            cursor = int((root / 'cursor').read_text()) if (root / 'cursor').exists() else len(text)
            x, y = (2, 13) if cursor == 0 else snapshot['prefix_end']
    if (root / 'late-home').exists():
        count_path = root / 'cursor-reads'
        count = int(count_path.read_text()) + 1 if count_path.exists() else 1
        count_path.write_text(str(count))
        if count < 3: x, y = 3, 1
    if (root / 'cursor-not-at-start').exists(): x, y = 3, 1
    if (root / 'wrong-prefix-end').exists() and (root / 'right-sent').exists(): x += 1
    print(x, y)

if command == 'capture-pane' and 'display-message' in args: atexit.register(print_cursor)
def append_draft_after_capture():
    if (root / 'replace-after-paste').exists(): return
    if (root / 'draft-after-exact').exists() and (root / 'paste-completed').exists() and not (root / 'draft-injected').exists():
        with (root / 'composer').open('a') as out: out.write((root / 'draft-text').read_text() if (root / 'draft-text').exists() else ' user addition')
        (root / 'draft-injected').touch()
if command == 'capture-pane': atexit.register(append_draft_after_capture)
if command == 'has-session':
    if (root / 'entered').exists() and (root / 'exit-after-enter').exists():
        if (root / 'exit-pending-pane').exists() and not (root / 'exit-pending-captured').exists(): sys.exit(0)
        if (root / 'probe-unknown-after-enter').exists():
            print('permission denied', file=sys.stderr); sys.exit(1)
        print("can't find session: modeled-guarded", file=sys.stderr); sys.exit(1)
    if (root / 'start-turn').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
    sys.exit(0)
if command == 'show-environment':
    commands = []
    for arg in args:
        if arg == ';': commands.append([])
        elif not commands: commands = [[arg]]
        else: commands[-1].append(arg)
    for row in commands:
        key = row[-1]
        values = {'AGENT_SESSION_ID': 'guarded', 'AGENT_SESSION_RUNTIME_ID': 'runtime-one', 'AGENT_SESSION_STATE_DIR': str(root / 'state')}
        if key == 'AGENT_SESSION_RUNTIME_ID':
            path = root / 'runtime-read-count'
            count = int(path.read_text()) + 1 if path.exists() else 1
            path.write_text(str(count))
            if (root / 'flip-second-runtime-read').exists() and count == 2: values[key] = 'runtime-replaced'
            if (root / 'runtime-replaced').exists(): values[key] = 'runtime-replaced'
        if key not in values: sys.exit(1)
        print(key + '=' + values[key])
    sys.exit(0)
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
    if (root / 'draft-on-load').exists(): (root / 'composer').write_text('new user draft')
    if (root / 'capture-fails-on-load').exists(): (root / 'capture-fails').touch()
    if (root / 'start-on-load').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
elif command == 'paste-buffer':
    if (root / 'draft-before-paste').exists(): (root / 'composer').write_text('hello world ')
    if (root / 'late-render').exists(): (root / 'late').write_text((root / 'buffer').read_text())
    else:
        with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
    if (root / 'fail-capture-after-paste').exists(): (root / 'capture-fails').touch()
    if (root / 'inject-draft').exists() and not (root / 'draft-after-exact').exists():
        with (root / 'composer').open('a') as out: out.write((root / 'draft-text').read_text() if (root / 'draft-text').exists() else ' user addition')
    (root / 'paste-completed').touch()
    if (root / 'replace-after-paste').exists(): (root / 'composer').write_text('replacement draft')
    if (root / 'start-after-paste').exists():
        p = root / 'state/sessions/guarded/activity.json'
        d = json.loads(p.read_text()); d['state']['phase'] = 'working'; d['state']['revision'] += 1
        p.write_text(json.dumps(d))
elif command == 'display-message':
    print_cursor()
elif command == 'send-keys' and args[-1] == 'Right':
    cursor = int((root / 'cursor').read_text())
    count = int(args[args.index('-N') + 1])
    (root / 'cursor').write_text(str(min(len((root / 'composer').read_text()), cursor + count)))
    (root / 'right-sent').touch()
elif command == 'send-keys' and args[-1] == 'C-a':
    (root / 'cursor').write_text('0')
    if (root / 'draft-on-home').exists(): (root / 'composer').write_text('replacement draft')
    if (root / 'replace-on-home').exists(): (root / 'runtime-replaced').touch()
elif command == 'send-keys' and args[-1] in ('BSpace', 'DC'):
    if (root / 'backspace-stalls').exists():
        (root / 'capture-fails').touch()
        import time
        time.sleep(3)
    if not (root / 'backspace-ignored').exists():
        count = int(args[args.index('-N') + 1]) if '-N' in args else 1
        text = (root / 'composer').read_text()
        cursor = int((root / 'cursor').read_text()) if (root / 'cursor').exists() else len(text)
        if args[-1] == 'DC': text = text[:cursor] + text[cursor + count:]
        else: text = text[:max(0, cursor - count)] + text[cursor:]
        (root / 'composer').write_text(text)
        if (root / 'redraw-after-delete').exists(): (root / 'redraw-pending').touch()
elif command == 'send-keys' and args[-1] == 'Enter':
    if (root / 'exit-after-enter').exists():
        (root / 'entered').touch(); sys.exit(0)
    if (root / 'unverified-after-enter').exists():
        (root / 'entered').touch(); sys.exit(0)
    with (root / 'submissions').open('a') as out: out.write(json.dumps((root / 'composer').read_text()) + '\n')
    (root / 'composer').write_text('')
    if (root / 'replace-after-enter').exists(): (root / 'runtime-replaced').touch()
elif command == 'capture-pane':
    if (root / 'exit-after-enter').exists() and (root / 'entered').exists():
        if (root / 'exit-pending-pane').exists() and not (root / 'exit-pending-captured').exists():
            (root / 'exit-pending-captured').touch()
        else: sys.exit(1)
    if (root / 'unverified-after-enter').exists() and (root / 'entered').exists() and '-e' not in args:
        print('provider view unavailable'); sys.exit(0)
    if (root / 'redraw-pending').exists():
        (root / 'redraw-pending').unlink()
        sys.exit(1)
    if (root / 'late').exists():
        # A slow renderer shows the paste only from the third capture after it.
        n = int((root / 'late-n').read_text()) + 1 if (root / 'late-n').exists() else 1
        (root / 'late-n').write_text(str(n))
        if n >= 3:
            with (root / 'composer').open('a') as out: out.write((root / 'late').read_text())
            (root / 'late').unlink()
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
    if (root / 'captured-layout').exists():
        snapshot = json.loads((root / 'captured-layout').read_text())
        if text == snapshot['input']:
            print(snapshot['pane'])
            sys.exit(0)
    # Native capture trims occupied spaces without -N; -T excludes unused cells.
    occupied = text if '-N' in args else text.rstrip(' ')
    padding = ' ' * 8 if '-N' in args and '-T' not in args else ''
    native = (root / 'native-layout').exists()
    rows = ''
    # An 80-column pane leaves 78 cells after the prompt/continuation prefix.
    if (root / 'wrap-80').exists() and len(occupied) > 78:
        chunks = [occupied[i:i + 78] for i in range(0, len(occupied), 78)]
        if (root / 'word-wrap').exists():
            chunks = textwrap.wrap(occupied, width=78, drop_whitespace=False)
            # Preserve spaces belonging to the logical input as well as padding.
            chunks = [row.ljust(78) for row in chunks[:-1]] + chunks[-1:]
        occupied = chunks[0]
        rows = ''.join('  ' + row + (' ' * (78 - len(row)) if provider == 'claude' else '') + '\n' for row in chunks[1:])
    footer = '\x1b]8;;https://example.invalid/\x1b\\status\x1b]8;;\x1b\\' if native else 'status'
    suggestion = (root / 'suggestion').read_text() if (root / 'suggestion').exists() else None
    top = '─── a long session title that fills the width of the pane ─' if (root / 'titled-border').exists() else '────────────────────────────'
    # Live Claude: a long title leaves no border dashes, and every composer row
    # is padded with written spaces that -N and -T keep.
    if (root / 'long-title').exists(): top = ' a deliberately long session title for the composer border lookup… ─'
    live = (root / 'live-layout').exists() and provider == 'claude'
    def pad(row):
        if live and (root / 'wrap-80').exists(): return row + ' ' * max(0, 78 - len(row))
        return row + ' ' * 20 if live else row
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
        // The model is a Python process per call; its shell fast path answers
        // the repeated default-state identity probes without starting Python.
        for (name, body) in [("tmux-model", script), ("tmux", TMUX_FAST_PATH)] {
            let path = temp.path().join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
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
fn guarded_send_post_paste_mismatch_preserves_user_input_without_submit() {
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

#[test]
fn guarded_send_unverified_submission_fails_cleans_and_retains_claim() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
        let guards = [
            "--require-empty-composer",
            "--idempotency-key",
            "uncertain-attempt",
        ];
        let output = fixture.send(&guards);
        let result = fixture.refused(&output, "send-submit-unconfirmed");
        let details = &result["error"]["details"];
        assert_eq!(details["outcome"], "unverified");
        assert_eq!(details["enter_presses"], 1);
        assert_eq!(details["pasted"], true);
        assert_eq!(details["cleared"], true);
        assert_eq!(details["remaining_input"], "empty");
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(
                    |call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter")
                )
                .count(),
            1
        );
        fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
    }
}
