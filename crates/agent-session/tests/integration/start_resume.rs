use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

#[test]
fn missing_resume_history_is_a_terminal_read_only_admission_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    for check in [true, false] {
        let state = tmp.path().join(if check { "check" } else { "start" });
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "claude",
            "--resume",
            "00000000-0000-4000-8000-000000000007",
            "--format",
            "json",
        ];
        if check {
            args.push("--check-resume");
        }
        let out = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("HOME", tmp.path().to_str().unwrap())
                .with_env(
                    "CLAUDE_CONFIG_DIR",
                    tmp.path().join("provider").to_str().unwrap(),
                ),
        );
        assert_ne!(out.code, 0);
        let response = out.stdout_json();
        assert_eq!(response["error"]["code"], "resume-history-missing");
        assert_eq!(response["error"]["details"]["session_created"], false);
        assert_eq!(response["error"]["details"]["safe_to_retry"], false);
        if check {
            assert_eq!(
                response["schema_version"],
                "cli.agent-session.resume-preflight.v1"
            );
        }
        assert!(
            !state.exists(),
            "refusal must not create even an operation or lock"
        );
    }
}

#[test]
fn malformed_resume_and_unsupported_provider_are_typed_before_creation() {
    let tmp = tempfile::tempdir().unwrap();
    for (agent, id, code) in [
        ("claude", "../invalid", "resume-unsupported"),
        (
            "codex",
            "00000000-0000-4000-8000-000000000007",
            "resume-unsupported",
        ),
    ] {
        let state = tmp.path().join(agent);
        let out = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "start",
                "--agent",
                agent,
                "--resume",
                id,
                "--check-resume",
                "--format",
                "json",
            ],
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        assert_eq!(out.stdout_json()["error"]["code"], code);
        assert!(!state.exists());
    }
}

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const CONVERSATION: &str = "00000000-0000-4000-8000-000000000007";

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

struct Fixture {
    tmp: tempfile::TempDir,
    state: PathBuf,
    root: PathBuf,
    broker: PathBuf,
    tmux: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let root = tmp.path().join("account");
        fs::create_dir_all(state.join("sessions/source")).unwrap();
        fs::create_dir_all(state.join("session-locks")).unwrap();
        fs::create_dir_all(root.join("projects/project")).unwrap();
        fs::write(root.join(".credentials.json"), "{}").unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(state.join("session-locks/source.lock"), "").unwrap();
        let history = json!({"sessionId":CONVERSATION,"cwd":tmp.path(),"type":"user"});
        fs::write(
            root.join(format!("projects/project/{CONVERSATION}.jsonl")),
            format!("{history}\n"),
        )
        .unwrap();
        let broker = tmp.path().join("broker");
        executable(
            &broker,
            &format!(
                r#"#!/bin/sh
case "$1" in
list) printf '%s\n' '{{"schema_version":"agent-session.account-broker.v2","provider":"claude","accounts":[{{"account":"original"}}]}}' ;;
materialize) printf '%s\n' '{}' ;;
*) exit 64 ;;
esac
"#,
                json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":root})
            ),
        );
        let tmux = tmp.path().join("tmux");
        executable(
            &tmux,
            "#!/bin/sh\nprintf 'no server running on fixture\\n' >&2\nexit 1\n",
        );
        let f = Self {
            tmp,
            state,
            root,
            broker,
            tmux,
        };
        f.write_source("source", "original");
        f
    }
    fn record_path(&self, id: &str) -> PathBuf {
        self.state.join("sessions").join(id).join("session.json")
    }
    fn write_source(&self, id: &str, account: &str) {
        fs::create_dir_all(self.record_path(id).parent().unwrap()).unwrap();
        fs::write(self.state.join(format!("session-locks/{id}.lock")), "").unwrap();
        let record = json!({"schema_version":"agent-session.session.v1", "id":id,"agent":"claude","mode":"interactive",
            "cwd":self.tmp.path(),"tmux_session":format!("hs-claude-{id}"),"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z",
            "provider_resume":{"provider":"claude","session_id":CONVERSATION,"captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":["--resume",CONVERSATION]},
            "runtime":{"kind":"tmux","tmux_session":format!("hs-claude-{id}"),"generation":1,"started_at":"2026-01-01T00:00:00Z","launch_id":"source-runtime"},
            "tmux_runtime_never_launched":"source-runtime",
            "claude_account_binding":{"schema_version":"agent-session.claude-account-binding.v1","selected_account":account,"selection_source":"explicit","revision":1,"state":"bound","applied_runtime_id":"source-runtime","config_dir":self.root,"updated_at":"2026-01-01T00:00:00Z"}});
        fs::write(self.record_path(id), record.to_string()).unwrap();
    }
    fn run(&self, extra: &[&str]) -> nils_test_support::cmd::CmdOutput {
        let options = CmdOptions::new()
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("HOME", self.tmp.path().to_str().unwrap())
            .with_env(
                "CLAUDE_CONFIG_DIR",
                self.tmp.path().join("unrelated-default").to_str().unwrap(),
            )
            .with_env(
                "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
                &json!([self.broker]).to_string(),
            )
            .with_env("AGENT_SESSION_TMUX_BIN", self.tmux.to_str().unwrap())
            .with_env(
                "AGENT_HOME",
                self.tmp.path().join("agent-home").to_str().unwrap(),
            )
            .with_env(
                "AGENT_SESSION_CLAUDE_BIN",
                self.tmp.path().join("provider").to_str().unwrap(),
            )
            .with_env(
                "AGENT_SESSION_FAKE_TMUX_LOG",
                self.tmp.path().join("tmux.log").to_str().unwrap(),
            )
            .with_env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1");
        let mut args = vec![
            "--state-dir",
            self.state.to_str().unwrap(),
            "--host",
            "test-machine",
            "start",
            "--agent",
            "claude",
            "--resume",
            CONVERSATION,
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        run_resolved("agent-session", &args, &options)
    }
}

#[test]
fn preflight_resolves_original_account_root_without_prompt_or_state_writes() {
    let f = Fixture::new();
    let before = fs::read(f.record_path("source")).unwrap();
    let out = f.run(&["--check-resume", "--prompt-file", "/nonexistent/prompt"]);
    assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
    assert_eq!(
        out.stdout_json(),
        json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
        "data":{"agent":"claude","provider_session_id":CONVERSATION,"machine":"test-machine","account":"original"}})
    );
    assert_eq!(fs::read(f.record_path("source")).unwrap(), before);
    assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
    assert!(!f.state.join("coordination").exists());
}

#[test]
fn pending_switch_preflight_requires_an_explicit_account_without_settling_it() {
    let f = Fixture::new();
    for state in ["queued", "failed"] {
        let mut source: Value =
            serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
        source["claude_account_next"] = json!({
            "schema_version":"agent-session.claude-account-next.v1",
            "account":"other", "revision":2, "state":state,
            "updated_at":"2026-01-01T00:00:00Z"
        });
        if state == "failed" {
            source["claude_account_next"]["failure_reason"] = json!("launch-failed");
        }
        fs::write(f.record_path("source"), source.to_string()).unwrap();
        let before = fs::read(f.record_path("source")).unwrap();
        assert_eq!(
            f.run(&["--check-resume"]).stdout_json()["error"]["code"],
            "resume-account-transition-pending"
        );
        assert_eq!(
            f.run(&["--check-resume", "--account", "other"])
                .stdout_json()["error"]["code"],
            "resume-account-mismatch"
        );
        let out = f.run(&["--check-resume", "--account", "original"]);
        assert_eq!(out.code, 0, "{}", out.stdout_text());
        assert_eq!(out.stdout_json()["data"]["account"], "original");
        assert_eq!(fs::read(f.record_path("source")).unwrap(), before);
        assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
    }
}

#[test]
fn all_sources_account_cwd_history_and_liveness_are_admission_guards() {
    let f = Fixture::new();
    assert_eq!(
        f.run(&["--check-resume", "--account", "other"])
            .stdout_json()["error"]["code"],
        "resume-account-mismatch"
    );
    assert_eq!(
        f.run(&["--check-resume", "--cwd", "/"]).stdout_json()["error"]["code"],
        "resume-unsupported"
    );
    f.write_source("second", "different");
    assert_eq!(
        f.run(&["--check-resume"]).stdout_json()["error"]["code"],
        "resume-source-unknown"
    );
    f.write_source("second", "original");
    executable(&f.tmux, "#!/bin/sh\nexit 0\n");
    assert_eq!(
        f.run(&["--check-resume"]).stdout_json()["error"]["code"],
        "resume-source-live"
    );
    executable(&f.tmux, "#!/bin/sh\nexit 2\n");
    assert_eq!(
        f.run(&["--check-resume"]).stdout_json()["error"]["code"],
        "resume-source-unknown"
    );
    fs::remove_file(
        f.root
            .join(format!("projects/project/{CONVERSATION}.jsonl")),
    )
    .unwrap();
    assert_eq!(
        f.run(&["--check-resume"]).stdout_json()["error"]["code"],
        "resume-history-missing"
    );
}

#[test]
fn start_creates_a_new_record_with_original_conversation_and_normal_context() {
    let f = Fixture::new();
    super::cli::fake_tmux(f.tmp.path());
    super::cli::fake_agent(f.tmp.path(), "provider");
    let source = fs::read(f.record_path("source")).unwrap();
    let out = f.run(&[
        "--id",
        "replacement",
        "--no-parent",
        "--role",
        "coordinator",
        "--workspace",
        "example",
        "--issue",
        "example/project#7",
        "--title",
        "Resume fixture",
        "--title-mode",
        "pinned",
        "--agent-arg=--model",
        "--agent-arg=fixture-model",
        "--prompt",
        "fixture bootstrap",
        "--paste-delay-ms",
        "0",
    ]);
    assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
    let response = out.stdout_json();
    assert_eq!(response["schema_version"], "cli.agent-session.start.v1");
    // Direct start preserves its ordinary session-view data shape.
    assert_eq!(response["data"]["session"]["id"], "replacement");
    let created: Value =
        serde_json::from_slice(&fs::read(f.record_path("replacement")).unwrap()).unwrap();
    assert_eq!(created["provider_resume"]["session_id"], CONVERSATION);
    assert_eq!(created["role"], "coordinator");
    assert_eq!(created["title_mode"], "pinned");
    assert_eq!(created["workspace"], "example");
    assert_eq!(
        created["claude_account_binding"]["selected_account"],
        "original"
    );
    assert_eq!(created["work"]["issues"][0]["number"], 7);
    assert_eq!(fs::read(f.record_path("source")).unwrap(), source);
    let calls = fs::read(f.tmp.path().join("tmux.log")).unwrap();
    let calls = String::from_utf8_lossy(&calls);
    assert!(calls.contains("--resume"));
    assert!(calls.contains(CONVERSATION));
    assert!(!calls.contains("--session-id"));
    assert!(calls.contains("fixture-model"));
    assert_eq!(
        fs::read_to_string(f.state.join("sessions/replacement/prompt.md")).unwrap(),
        "fixture bootstrap"
    );
    // A subsequent resume of the old managed ID must see the replacement's
    // writer and refuse instead of reopening a second runtime.
    let selective = f.tmp.path().join("selective-tmux");
    executable(
        &selective,
        "#!/bin/sh\ncase \"$*\" in *'=hs-claude-source'*) printf \"can't find session: source\\n\" >&2; exit 1;; *) exit 0;; esac\n",
    );
    let out = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            f.state.to_str().unwrap(),
            "resume",
            "source",
            "--tmux-bin",
            selective.to_str().unwrap(),
            "--format",
            "json",
        ],
        &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
    );
    assert_eq!(
        out.stdout_json()["error"]["code"],
        "resume-source-live",
        "{}",
        out.stdout_text()
    );
    assert_eq!(fs::read(f.record_path("source")).unwrap(), source);
}

#[test]
fn busy_source_and_history_race_refuse_before_new_record() {
    use std::os::fd::AsRawFd;
    let f = Fixture::new();
    let lock = fs::File::open(f.state.join("session-locks/source.lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert_eq!(
        f.run(&["--check-resume"]).stdout_json()["error"]["code"],
        "resume-source-live"
    );
    drop(lock);
    assert_eq!(f.run(&["--check-resume"]).code, 0);
    // Materialization is start-only and may race history cleanup. The shared
    // import resolver must recheck before ordinary record creation.
    let history = f
        .root
        .join(format!("projects/project/{CONVERSATION}.jsonl"));
    let script = fs::read_to_string(&f.broker).unwrap().replace(
        "materialize)",
        &format!(
            "materialize) rm -- {};",
            shell_words::quote(history.to_str().unwrap())
        ),
    );
    executable(&f.broker, &script);
    let out = f.run(&["--id", "must-not-exist"]);
    assert_eq!(
        out.stdout_json()["error"]["code"],
        "resume-history-missing",
        "{}",
        out.stdout_text()
    );
    assert_eq!(
        out.stdout_json()["error"]["details"]["session_created"],
        false
    );
    assert_eq!(
        out.stdout_json()["error"]["details"]["safe_to_retry"],
        false
    );
    assert!(!f.record_path("must-not-exist").exists());
}

#[test]
fn auth_quarantine_is_not_an_alternate_resume_api() {
    let f = Fixture::new();
    fs::write(f.state.join("sessions/source/auth-incidents.json"), json!({
        "schema_version":"agent-session.auth-incidents.v1", "incidents":[{
            "incident_id":"failure", "session_id":"source", "runtime_incarnation":"source-runtime", "runtime_generation":1,
            "provider":"claude", "account_nickname":"original", "observed_at":"2026-01-01T00:00:00Z",
            "source":"terminal_pattern", "confidence":"observed", "status":"auth_failed", "recovery_result":null,
            "recovery_observed_at":null, "notification":{}, "recovery_notification":{}
        }], "last_failure_key":null, "detection_health":null
    }).to_string()).unwrap();
    let out = f.run(&["--id", "blocked"]);
    assert_eq!(out.stdout_json()["error"]["code"], "resume-source-unknown");
    assert!(!f.record_path("blocked").exists());
    let path = f.state.join("sessions/source/auth-incidents.json");
    let mut store: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    store["incidents"][0]["status"] = json!("recovered");
    fs::write(path, store.to_string()).unwrap();
    assert_eq!(
        f.run(&["--check-resume"]).code,
        0,
        "a valid settled incident is readable"
    );
}

#[test]
fn held_conversation_lock_is_a_terminal_precreate_refusal() {
    use sha2::{Digest, Sha256};
    use std::os::fd::AsRawFd;
    let f = Fixture::new();
    let root = f.state.join("coordination");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let digest = Sha256::digest(format!("claude:{CONVERSATION}").as_bytes());
    let digest = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let lock = fs::File::create(root.join(format!("provider-resume-{digest}.lock"))).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let before = fs::read(f.record_path("source")).unwrap();
    let out = f.run(&["--id", "blocked-by-lock"]);
    assert_eq!(
        out.stdout_json(),
        json!({
            "schema_version":"cli.agent-session.start.v1", "ok":false,
            "error":{"code":"resume-source-live", "message":"a managed source conversation still has a live or busy writer",
                "details":{"session_created":false,"safe_to_retry":false}}
        })
    );
    assert!(!f.record_path("blocked-by-lock").exists());
    assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
    assert_eq!(fs::read(f.record_path("source")).unwrap(), before);
}

#[path = "start_resume_console.rs"]
mod console;
