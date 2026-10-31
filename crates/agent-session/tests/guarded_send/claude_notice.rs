//! Modeled CLI boundaries; these tests do not execute a native Claude runtime.
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

struct Pane(std::process::Child);

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn write_executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn launch(root: &Path, gate: &str, version: &str) -> (CmdOutput, Vec<Vec<String>>) {
    launch_case(root, gate, version, false)
}

fn launch_case(
    root: &Path,
    gate: &str,
    version: &str,
    resume: bool,
) -> (CmdOutput, Vec<Vec<String>>) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let pane = Pane(
        Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let tmux = root.join("launch-tmux");
    let log = root.join("launch-calls");
    write_executable(
        &tmux,
        &format!(
            r#"#!/usr/bin/env python3
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'launch-calls').open('a') as out:
    out.write(json.dumps(args) + '\n')
command = args[0]
if command == 'has-session':
    if (root / 'launch-live').exists(): sys.exit(0)
    print('no server running on fixture', file=sys.stderr)
    sys.exit(1)
if command == 'new-session':
    (root / 'launch-live').touch()
    values = {{}}
    for i, arg in enumerate(args[:-1]):
        if arg == '-e' and '=' in args[i + 1]:
            key, value = args[i + 1].split('=', 1)
            values[key] = value
    (root / 'launch-env.json').write_text(json.dumps(values))
    for i, arg in enumerate(args):
        if arg.endswith('/coordination/heartbeat'):
            heartbeat = pathlib.Path(arg)
            heartbeat.parent.mkdir(parents=True, exist_ok=True)
            heartbeat.write_text(args[i + 2] + ':' + str(int(time.time())) + '\n')
            heartbeat.chmod(0o600)
    print('$42\t%42\t{pid}')
elif command == 'display-message':
    text = args[-1]
    for key, value in [('session_id', '$42'), ('pane_id', '%42'), ('pane_pid', '{pid}'), ('pane_dead', '0'), ('pane_dead_status', '0')]:
        text = text.replace('#{{' + key + '}}', value)
    print(text)
elif command == 'show-environment':
    values = json.loads((root / 'launch-env.json').read_text())
    key = args[-1]
    if key not in values: sys.exit(1)
    print(key + '=' + values[key])
elif command == 'kill-session':
    (root / 'launch-live').unlink(missing_ok=True)
"#,
            pid = pane.0.id()
        ),
    );
    let agent = root.join("modeled-claude");
    write_executable(
        &agent,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' '{version} (Claude Code)'; fi\n"
        ),
    );
    let provider = root.join("provider-config");
    fs::create_dir_all(&provider).unwrap();
    let state = root.join("state");
    if resume {
        let session = state.join("sessions/synthetic");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("session.json"), json!({
            "schema_version":"agent-session.session.v1", "id":"synthetic", "agent":"claude",
            "mode":"interactive", "cwd":root, "tmux_session":"hs-claude-synthetic",
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "agent_bin":agent,
            "provider_resume":{"provider":"claude", "session_id":"provider-one", "captured_at":"2026-01-01T00:00:00Z",
                "capture_method":"fixture", "resume_args":["--resume","provider-one"]},
            "runtime":{"kind":"tmux", "tmux_session":"hs-claude-synthetic", "generation":1,
                "started_at":"2026-01-01T00:00:00Z", "launch_id":"never-launched-fixture"},
            "tmux_runtime_never_launched":"never-launched-fixture"
        }).to_string()).unwrap();
    }
    let args = if resume {
        vec![
            "--state-dir",
            state.to_str().unwrap(),
            "resume",
            "synthetic",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ]
    } else {
        vec![
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "claude",
            "--id",
            "synthetic",
            "--cwd",
            root.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ]
    };
    let output = run_resolved(
        "agent-session",
        &args,
        &CmdOptions::new()
            .with_cwd(root)
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("AGENT_SESSION_NOTICE_WRITERS", gate)
            .with_env("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap())
            .with_env("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "100")
            .with_env("AGENT_HOME", root.join("agent-home").to_str().unwrap())
            .with_env("CLAUDE_CONFIG_DIR", provider.to_str().unwrap())
            .with_env("CODEX_HOME", root.join("unused-codex").to_str().unwrap())
            .with_env("XDG_RUNTIME_DIR", root.to_str().unwrap()),
    );
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    let calls = fs::read(log)
        .unwrap()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    (output, calls)
}

fn mounted_module(calls: &[Vec<String>]) -> Option<PathBuf> {
    calls
        .iter()
        .find(|call| call.first().is_some_and(|arg| arg == "new-session"))?
        .windows(2)
        .find(|args| args[0] == "--plugin-dir")
        .map(|args| PathBuf::from(&args[1]))
}

#[test]
fn claude_notice_supported_writer_launch_mounts_native_module() {
    for resume in [false, true] {
        let temp = tempfile::tempdir_in("/tmp").unwrap();
        let (output, calls) = launch_case(temp.path(), "1", "2.1.296", resume);
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        let module = mounted_module(&calls).expect(
            "a supported writer-enabled fresh Claude launch must mount its native notice module",
        );
        assert!(module.starts_with(temp.path().join("state/sessions/synthetic")));
        for path in [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/register.js",
            "bootstrap.json",
            "ready.json",
            "offer.json",
        ] {
            assert!(
                module.join(path).is_file(),
                "missing mounted module file: {path}"
            );
        }
        let boot: Value =
            serde_json::from_slice(&fs::read(module.join("bootstrap.json")).unwrap()).unwrap();
        if resume {
            assert_eq!(boot["launch"]["generation"], 2);
            assert_ne!(boot["launch"]["incarnation"], "never-launched-fixture");
            assert!(calls.iter().any(|call| {
                call.windows(2)
                    .any(|args| args == ["--resume", "provider-one"])
            }));
        }
    }
}

#[test]
fn claude_notice_gate_off_and_other_versions_do_not_mount_a_module() {
    for (gate, version) in [("0", "2.1.296"), ("1", "2.1.295")] {
        for resume in [false, true] {
            let temp = tempfile::tempdir_in("/tmp").unwrap();
            let (output, calls) = launch_case(temp.path(), gate, version, resume);
            assert_eq!(
                output.code,
                0,
                "{} {}",
                output.stdout_text(),
                output.stderr_text()
            );
            assert_eq!(mounted_module(&calls), None);
        }
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bind_modeled_peer(fixture: &super::Fixture) -> (PathBuf, Value) {
    let root = fixture.root();
    let (_, calls) = launch(root, "1", "2.1.296");
    let installed = mounted_module(&calls).unwrap();
    let dir = root.join("state/sessions/guarded/claude-notice/runtime-one");
    fs::create_dir_all(dir.join("hooks")).unwrap();
    for path in [
        dir.parent().unwrap(),
        dir.as_path(),
        dir.join("hooks").as_path(),
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut boot: Value =
        serde_json::from_slice(&fs::read(installed.join("bootstrap.json")).unwrap()).unwrap();
    boot["launch"] =
        json!({"session_id":"guarded","incarnation":"runtime-one","generation":1,"profile":null});
    let state = root.join("state");
    let namespace = hash(format!("{}\0guarded\0runtime-one", state.display()).as_bytes());
    boot["socket_path"] = json!(root.join(format!("agent-session/cn-{}.sock", &namespace[..16])));
    let binding = json!({"launch":boot["launch"],"provider":"claude","provider_session_id":"provider-one",
        "provider_version":"2.1.296","module_sha256":boot["module_sha256"]});
    let ready = json!({"schema_version":"agent-session.claude-notice-ready.v1","binding":binding,
        "nonce":boot["nonce"],"terminal":true,"timer_started":true});
    for (name, bytes) in [
        ("bootstrap.json", serde_json::to_vec(&boot).unwrap()),
        ("ready.json", serde_json::to_vec(&ready).unwrap()),
        (
            "offer.json",
            b"{\"schema_version\":\"agent-session.claude-notice-offer.v1\",\"state\":\"idle\"}"
                .to_vec(),
        ),
        (
            "hooks/register.js",
            fs::read(installed.join("hooks/register.js")).unwrap(),
        ),
    ] {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let provider = root.join("provider-config");
    let transcript = provider.join("projects/synthetic/provider-one.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({"type":"summary","sessionId":"provider-one","cwd":root})
        ),
    )
    .unwrap();
    let record_path = root.join("state/sessions/guarded/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["agent_profile_provider_config_dir"] = json!(provider);
    fs::write(record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    (dir, boot)
}

fn post(socket: &Path, path: &str, value: &Value) -> Option<Value> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .ok()?;
    let body = value.to_string();
    write!(stream, "POST {path} HTTP/1.1\r\nHost: notice.invalid\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).ok()?;
    let mut response = String::new();
    stream.take(128 * 1024).read_to_string(&mut response).ok()?;
    let (head, body) = response.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200 ") {
        return None;
    }
    serde_json::from_str(body).ok()
}

struct Peer {
    stop: Arc<AtomicBool>,
    text: Arc<Mutex<Vec<String>>>,
    consumed_before_text: Arc<AtomicBool>,
    hook_unlocked: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Peer {
    fn start(fixture: &super::Fixture, mode: &'static str) -> Self {
        let (dir, boot) = bind_modeled_peer(fixture);
        let root = fixture.root().to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let text = Arc::new(Mutex::new(Vec::new()));
        let consumed = Arc::new(AtomicBool::new(false));
        let hook_unlocked = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(mode != "hold"));
        let stopped = stop.clone();
        let observed = text.clone();
        let claimed = consumed.clone();
        let hook = hook_unlocked.clone();
        let released = release.clone();
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let offer = loop {
                if stopped.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    return;
                }
                if let Ok(bytes) = fs::read(dir.join("offer.json"))
                    && let Ok(offer) = serde_json::from_slice::<Value>(&bytes)
                    && offer["op"].is_string()
                {
                    break offer;
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            let request = json!({"schema_version":"agent-session.claude-notice-authorize.v1", "op":offer["op"],
                "binding":offer["binding"], "nonce":boot["nonce"], "terminal":true,"box_empty":true,"cursor":0});
            let socket = Path::new(boot["socket_path"].as_str().unwrap());
            let Some(authorized) = post(socket, "/authorize", &request) else {
                return;
            };
            let Some(value) = authorized["text"].as_str() else {
                return;
            };
            observed.lock().unwrap().push(value.to_string());
            let keys_exist = fs::read_dir(root.join("state/sessions/guarded"))
                .unwrap()
                .any(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("send-key-")
                });
            claimed.store(
                keys_exist
                    && root
                        .join("state/history/claude-notices/ledger.jsonl")
                        .is_file(),
                Ordering::SeqCst,
            );
            while !released.load(Ordering::SeqCst)
                && !stopped.load(Ordering::SeqCst)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            if mode == "lost-result" {
                return;
            }
            let lock = fs::File::open(root.join("state/session-locks/guarded.lock")).unwrap();
            // SAFETY: this fixture owns the descriptor; the real CLI must have
            // released its record lock before the modeled provider hook runs.
            let unlocked =
                unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
            hook.store(unlocked, Ordering::SeqCst);
            if unlocked {
                let record_path = root.join("state/sessions/guarded/session.json");
                let mut record: Value =
                    serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
                record["modeled_native_hook"] = json!(true);
                fs::write(record_path, record.to_string()).unwrap();
                // SAFETY: the fixture still owns the descriptor and its lock.
                unsafe {
                    libc::flock(lock.as_raw_fd(), libc::LOCK_UN);
                }
            }
            if mode == "draft-after" {
                fs::write(root.join("composer"), "new user draft").unwrap();
            }
            if mode == "identity-change" {
                fs::write(root.join("runtime-replaced"), "").unwrap();
            }
            if matches!(mode, "dropped" | "draft-after") || !unlocked {
                let report = json!({"schema_version":"agent-session.claude-notice-result.v1", "op":offer["op"],
                    "binding":offer["binding"], "nonce":boot["nonce"],
                    "outcome":if mode == "dropped" {"dropped"} else {"unknown"}, "after_empty":mode != "draft-after"});
                let _ = post(socket, "/result", &report);
                return;
            }
            let transcript = root.join("provider-config/projects/synthetic/provider-one.jsonl");
            let row = json!({"type":"user","sessionId":"provider-one","uuid":"11111111-1111-4111-8111-111111111111",
                "version":"2.1.296","promptSource":"system","isSidechain":false,"cwd":root,
                "origin":{"kind":"plugin","name":"agent-session-claude-notice","asUser":true},
                "message":{"role":"user","content":[{"type":"text","text":value}]}});
            writeln!(
                fs::OpenOptions::new()
                    .append(true)
                    .open(transcript)
                    .unwrap(),
                "{row}"
            )
            .unwrap();
            let result = json!({"schema_version":"agent-session.claude-notice-result.v1", "op":offer["op"],
                "binding":offer["binding"], "nonce":boot["nonce"], "outcome":"accepted", "after_empty":true,
                "returned_text":if mode == "wrong-result" {"rewritten input"} else {value},
                "origin":{"kind":"plugin","name":"agent-session-claude-notice","as_user":true}});
            let _ = post(socket, "/result", &result);
        });
        Self {
            stop,
            text,
            consumed_before_text: consumed,
            hook_unlocked,
            release,
            thread: Some(thread),
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn send_notice(fixture: &super::Fixture, body: &str) -> CmdOutput {
    send_notice_key(fixture, body, "claude-native-once")
}

fn send_notice_key(fixture: &super::Fixture, body: &str, key: &str) -> CmdOutput {
    let root = fixture.root();
    fs::write(root.join("lf-body"), body).unwrap();
    run_resolved(
        "agent-session",
        &[
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "notice",
            "guarded",
            "--kind",
            "mailbox",
            "--source",
            "synthetic",
            "--body-file",
            root.join("lf-body").to_str().unwrap(),
            "--tmux-bin",
            root.join("tmux").to_str().unwrap(),
            "--idempotency-key",
            key,
            "--format",
            "json",
        ],
        &CmdOptions::new()
            .with_cwd(root)
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("AGENT_SESSION_NOTICE_WRITERS", "1")
            .with_env("XDG_RUNTIME_DIR", root.to_str().unwrap())
            .with_env(
                "CLAUDE_CONFIG_DIR",
                root.join("provider-config").to_str().unwrap(),
            ),
    )
}

#[test]
fn claude_notice_supported_native_delivery_is_exact_and_consumed_once() {
    for body in [
        "Alpha  \nBeta \n".to_string(),
        (0..36).map(|i| format!("row {i}  \n")).collect(),
        "  本文 🦀\nFinal  \n".to_string(),
    ] {
        let fixture = super::Fixture::new("claude");
        let peer = Peer::start(&fixture, "accepted");
        let output = send_notice(&fixture, &body);
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        assert_eq!(
            output.stdout_json()["data"]["native"]["transport"],
            "claude_mod"
        );
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            0
        );
        let expected = format!("<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\n{body}");
        assert_eq!(*peer.text.lock().unwrap(), [expected]);
        assert!(peer.consumed_before_text.load(Ordering::SeqCst));
        assert!(peer.hook_unlocked.load(Ordering::SeqCst));
        let record: Value = serde_json::from_slice(
            &fs::read(fixture.root().join("state/sessions/guarded/session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record["modeled_native_hook"], true,
            "confirmation must not overwrite hook updates"
        );
        fixture.refused(&send_notice(&fixture, &body), "send-idempotency-key-used");
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
    }
}

#[test]
fn claude_notice_postclaim_failures_retain_key_and_preserve_unknown_draft() {
    for mode in [
        "dropped",
        "wrong-result",
        "draft-after",
        "identity-change",
        "lost-result",
    ] {
        let fixture = super::Fixture::new("claude");
        let peer = Peer::start(&fixture, mode);
        let output = send_notice(&fixture, "synthetic\n");
        let value = fixture.refused(&output, "notice-native-unconfirmed");
        assert_eq!(value["error"]["details"]["submitted"], Value::Null);
        assert_eq!(value["error"]["details"]["pasted"], false);
        fixture.refused(
            &send_notice(&fixture, "synthetic\n"),
            "send-idempotency-key-used",
        );
        assert_eq!(
            peer.text.lock().unwrap().len(),
            1,
            "{mode}: never release another body"
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            if mode == "draft-after" {
                "new user draft"
            } else {
                ""
            }
        );
    }
}

#[test]
fn claude_notice_native_cr_and_oversize_refuse_without_claim_or_body() {
    for body in ["Alpha\r\nBeta".to_string(), "x".repeat(16 * 1024)] {
        let fixture = super::Fixture::new("claude");
        let peer = Peer::start(&fixture, "accepted");
        let value = fixture.refused(
            &send_notice(&fixture, &body),
            "notice-native-text-unsupported",
        );
        assert_eq!(value["error"]["details"]["submitted"], false);
        assert!(peer.text.lock().unwrap().is_empty());
        assert!(
            !fs::read_dir(fixture.root().join("state/sessions/guarded"))
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("send-key-"))
        );
        fixture.no_delivery();
    }
}

#[test]
fn claude_notice_ledger_reserves_unfinished_pairs_and_refuses_partial_data() {
    for partial in [false, true] {
        let fixture = super::Fixture::new("claude");
        let peer = Peer::start(&fixture, "accepted");
        let root = fixture.root();
        let module = root.join("state/sessions/guarded/claude-notice/runtime-one");
        let ready: Value =
            serde_json::from_slice(&fs::read(module.join("ready.json")).unwrap()).unwrap();
        let parent = root.join("state/history/claude-notices");
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let ledger = parent.join("ledger.jsonl");
        let text = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nsynthetic\n";
        let bytes = if partial {
            "{\"kind\":\"claim\"".to_string()
        } else {
            (0..1024).map(|index| {
                let mut binding = ready["binding"].clone();
                binding["launch"]["session_id"] = json!(format!("other-{index}"));
                format!("{}\n", json!({
                "schema_version":"agent-session.claude-notice-ledger.v1", "kind":"claim",
                "op":format!("{index:08x}-0000-4000-8000-000000000000"),"binding":binding,
                "history_id":format!("h-{}",hash(b"claude\0\0provider-one")),
                "envelope_bytes":text.len(),"envelope_sha256":hash(text.as_bytes()),"original_budget_ms":15000,
                "baseline":{"device":0,"inode":0,"eof":0,"continuity_sha256":hash(b"")}
            }))}).collect()
        };
        assert!(
            bytes.len() < 8 * 1024 * 1024,
            "reservation limit must be distinct from file length"
        );
        fs::write(&ledger, &bytes).unwrap();
        fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();
        fixture.refused(
            &send_notice(&fixture, "synthetic\n"),
            "notice-native-unavailable",
        );
        assert_eq!(
            fs::read(&ledger).unwrap(),
            bytes.as_bytes(),
            "never repair or evict evidence"
        );
        assert!(peer.text.lock().unwrap().is_empty());
        fixture.no_delivery();
    }
}

#[test]
fn claude_notice_without_provider_transcript_refuses_distinctly_and_claims_nothing() {
    let fixture = super::Fixture::new("claude");
    let peer = Peer::start(&fixture, "accepted");
    let transcript = fixture
        .root()
        .join("provider-config/projects/synthetic/provider-one.jsonl");
    let bytes = fs::read(&transcript).unwrap();
    fs::remove_file(&transcript).unwrap();
    let value = fixture.refused(
        &send_notice(&fixture, "synthetic\n"),
        "notice-native-baseline-unavailable",
    );
    let details = &value["error"]["details"];
    assert_eq!(details["guard"], "baseline");
    assert_eq!(details["stage"], "admission");
    assert_eq!(details["pasted"], false);
    assert_eq!(details["submitted"], false);
    assert!(peer.text.lock().unwrap().is_empty());
    let sessions = fixture.root().join("state/sessions/guarded");
    assert!(!fs::read_dir(&sessions).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("send-key-")
    }));
    assert!(
        !fixture
            .root()
            .join("state/history/claude-notices/ledger.jsonl")
            .exists()
    );
    fixture.no_delivery();
    // The refusal claimed nothing, so the same key delivers once the provider
    // has recorded a turn.
    fs::write(&transcript, bytes).unwrap();
    let output = send_notice(&fixture, "synthetic\n");
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(peer.text.lock().unwrap().len(), 1);
    fixture.refused(
        &send_notice(&fixture, "synthetic\n"),
        "send-idempotency-key-used",
    );
}

#[test]
fn claude_notice_concurrent_attempt_is_busy_before_consumption() {
    let fixture = super::Fixture::new("claude");
    let peer = Peer::start(&fixture, "hold");
    std::thread::scope(|scope| {
        let first = scope.spawn(|| send_notice_key(&fixture, "synthetic\n", "first"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while peer.text.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let second = send_notice_key(&fixture, "synthetic\n", "second");
        peer.release.store(true, Ordering::SeqCst);
        let first = first.join().unwrap();
        fixture.refused(&second, "send-session-not-idle");
        assert_eq!(first.code, 0, "{}", first.stdout_text());
        assert_eq!(peer.text.lock().unwrap().len(), 1);
        let keys = fs::read_dir(fixture.root().join("state/sessions/guarded"))
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("send-key-")
            })
            .count();
        assert_eq!(keys, 1);
        fixture.no_delivery();
    });
}
