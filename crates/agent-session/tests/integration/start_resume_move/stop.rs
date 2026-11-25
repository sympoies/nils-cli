use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use agent_session::cli::resume_move::{STOP_PROOF_SCHEMA, SignedRecord, TICKET_SCHEMA, sha256};
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::json;

use super::support::{Signers, write_private};

const THREAD: &str = "00000000-0000-4000-8000-000000000031";
const INCARNATION: &str = "00000000-0000-4000-8000-000000000032";

#[test]
fn managed_codex_stop_retains_the_source_for_signed_export() {
    managed_stop_retains_the_source_for_signed_export("codex", false);
}

#[test]
fn managed_claude_stop_retains_the_source_for_signed_export() {
    managed_stop_retains_the_source_for_signed_export("claude", false);
}

#[test]
fn old_stop_receipt_cannot_authorize_export_after_replacement() {
    managed_stop_retains_the_source_for_signed_export("codex", true);
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

struct Runtime(Option<Child>);

impl Runtime {
    fn spawn(command: &mut Command) -> Self {
        Self(Some(command.spawn().unwrap()))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn managed_stop_retains_the_source_for_signed_export(agent: &str, replace_before_export: bool) {
    let ssh_keygen = Path::new("/usr/bin/ssh-keygen");
    if !ssh_keygen.is_file() {
        eprintln!("skipping stop-and-retain fixture: ssh-keygen unavailable");
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let state = root.join("state");
    let session = state.join("sessions/source");
    let provider = root.join("provider");
    private_dir(&state);
    private_dir(&state.join("sessions"));
    private_dir(&session);
    private_dir(&state.join("session-locks"));
    let coordination = state.join("coordination");
    private_dir(&coordination);
    write_private(&state.join("session-locks/source.lock"), b"");
    let history = if agent == "codex" {
        provider
            .join("sessions/2026/01/01")
            .join(format!("rollout-{THREAD}.jsonl"))
    } else {
        provider
            .join("projects/source-checkout")
            .join(format!("{THREAD}.jsonl"))
    };
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    let history_line = if agent == "codex" {
        json!({
            "type": "session_meta",
            "payload": {"id": THREAD, "cwd": root, "source": "cli",
                "timestamp": "2026-01-01T00:00:00Z", "history_mode": "legacy"}
        })
    } else {
        json!({
            "type":"user", "sessionId": THREAD, "cwd":root,
            "message":{"role":"user","content":"synthetic conversation"}
        })
    };
    let history_bytes = format!("{history_line}\n");
    write_private(&history, history_bytes.as_bytes());

    let mut provider_process = Command::new("/bin/sh");
    provider_process.current_dir(&root).args([
        "-c",
        "while [ ! -f exit-request ] || [ -f hold-exit ]; do sleep 0.02; done; cp stopped-registry state/coordination/registry.json",
    ]);
    // SAFETY: setsid is async-signal-safe and gives the synthetic provider a
    // process boundary of its own, disjoint from the test runner.
    unsafe {
        provider_process.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut runtime = Runtime::spawn(&mut provider_process);
    let pid = runtime.0.as_ref().unwrap().id();
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let start_time = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    fs::write(root.join("pane-pid"), pid.to_string()).unwrap();
    fs::write(root.join("agent"), agent).unwrap();
    let registry = |status| {
        json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "brokers": {"source": {
                "session_id": "source", "incarnation": INCARNATION, "generation": 1,
                "state": status, "capability_digest": "",
                "heartbeat_at": "2026-01-01T00:00:00Z", "heartbeat_epoch": 1
            }}
        })
    };
    write_private(
        &coordination.join("registry.json"),
        registry("ready").to_string().as_bytes(),
    );
    write_private(
        &root.join("stopped-registry"),
        registry("stopped").to_string().as_bytes(),
    );

    let record_path = session.join("session.json");
    let mut record = json!({
        "schema_version": "agent-session.session.v1", "id": "source", "agent": agent,
        "mode": "interactive", "cwd": root, "tmux_session": "fixture-source",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "provider_resume": {
            "provider": agent, "session_id": THREAD,
            "captured_at": "2026-01-01T00:00:00Z", "capture_method": "fixture",
            "resume_args": ["resume", THREAD]
        },
        "runtime": {"kind": "tmux", "tmux_session": "fixture-source", "generation": 1,
            "started_at": "2026-01-01T00:00:00Z", "launch_id": INCARNATION,
            "agent_profile_provider_config_dir": provider},
        "delete_tmux_identity": {
            "launch_id": INCARNATION, "session_id": "$7", "pane_id": "%8",
            "pane_pid": pid, "pane_start_time": start_time, "process_group_id":pid,
            "process_session_id":pid, "process_session_members": [{"pid":pid,"start_time":start_time}]
        }
    });
    record[format!("{agent}_account_binding")] = json!({
        "schema_version": format!("agent-session.{agent}-account-binding.v1"),
        "selected_account": "fixture-account", "revision": 1, "state": "bound",
        "config_dir": provider,
        "applied_runtime_id": INCARNATION, "updated_at": "2026-01-01T00:00:00Z"
    });
    let record_bytes = record.to_string().into_bytes();
    write_private(&record_path, &record_bytes);
    let activity = session.join("activity.json");
    let projected_thread = format!(
        "local:v1:{}",
        sha256(
            format!(
                "agent-session.provider-identifier.v1\0{INCARNATION}\0{agent}\0session\0{THREAD}"
            )
            .as_bytes()
        )
    );
    let activity_for = |phase| {
        json!({
            "schema_version": "agent-session.activity.v1",
            "runtime_id":INCARNATION, "runtime_generation":1, "provider_session_id":projected_thread,
            "seen_event_count":0,
            "state":{
                "schema_version":"agent-session.turn-state.v1", "phase":phase, "revision":2,
                "phase_changed_at":"2026-01-01T00:00:00Z",
                "source":{"kind":"provider_hook","confidence":"authoritative"}
            }
        })
    };
    write_private(&activity, activity_for("waiting").to_string().as_bytes());
    let signers = Signers::new(&root, ssh_keygen);
    let now = jiff::Timestamp::now().as_second();
    let payload = json!({
        "schema_version": TICKET_SCHEMA,
        "binding": {
            "move_id": "00000000-0000-4000-8000-000000000033",
            "nonce": "00000000-0000-4000-8000-000000000034",
            "source_host": "host-a", "destination_host": "host-b", "source_session_id": "source",
            "source_incarnation": INCARNATION, "provider": agent, "provider_session_id": THREAD,
            "account": "fixture-account", "destination_checkout_sha256": sha256(b"fixture destination")
        },
        "issued_at": now, "expires_at": now + 300
    }).to_string();
    let payload_path = root.join("ticket-payload");
    write_private(&payload_path, payload.as_bytes());
    assert!(
        Command::new(ssh_keygen)
            .args(["-Y", "sign", "-n", TICKET_SCHEMA, "-f"])
            .arg(&signers.b_key)
            .arg(&payload_path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let signature = fs::read_to_string(payload_path.with_extension("sig")).unwrap();
    let ticket = root.join("ticket.json");
    write_private(
        &ticket,
        &serde_json::to_vec(&SignedRecord { payload, signature }).unwrap(),
    );

    let tmux = root.join("tmux");
    fs::write(&tmux, TMUX).unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_env("HOME", root.to_str().unwrap())
        .with_env("CODEX_HOME", provider.to_str().unwrap())
        .with_env("AGENT_SESSION_CLAUDE_ACCOUNT_BROKER", r#"["/bin/false"]"#)
        .with_env("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap())
        .with_env("AGENT_SESSION_RESUME_MOVE_HOST", "host-a")
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN",
            ssh_keygen.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE",
            signers.allowlist.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE",
            signers.a_key.to_str().unwrap(),
        );
    let run = |args: &[&str]| {
        let mut command = vec!["--state-dir", state.to_str().unwrap(), "resume-move"];
        command.extend_from_slice(args);
        command.extend(["--format", "json"]);
        run_resolved("agent-session", &command, &options)
    };
    let export = || {
        run(&[
            "export",
            "--session",
            "source",
            "--incarnation",
            INCARNATION,
            "--destination",
            "host-b",
            "--ticket-file",
            ticket.to_str().unwrap(),
        ])
    };
    let live = export();
    assert_eq!(
        live.stdout_json()["error"]["code"],
        "resume-source-live",
        "{}",
        live.stdout_text()
    );
    assert!(
        !state
            .join("coordination/resume-move-source.v1.json")
            .exists()
    );

    let stop_args = ["stop", "--session", "source", "--incarnation", INCARNATION];
    let code = |args: &[&str]| run(args).stdout_json()["error"]["code"].clone();
    assert_eq!(
        code(&["stop", "--session", "source", "--incarnation", THREAD]),
        "resume-move-session-mismatch"
    );
    assert_eq!(
        code(&["stop", "--session", "so", "--incarnation", INCARNATION]),
        "resume-move-session-mismatch"
    );
    if agent == "codex" {
        write_private(
            &activity,
            activity_for("needs_input").to_string().as_bytes(),
        );
        assert_eq!(code(&stop_args), "resume-move-source-busy");
        write_private(&activity, activity_for("waiting").to_string().as_bytes());
        fs::write(root.join("draft"), b"draft stays").unwrap();
        let refusal = run(&stop_args).stdout_json();
        assert_eq!(refusal["error"]["code"], "resume-move-source-busy");
        assert_eq!(refusal["error"]["details"]["cause"], "composer-draft");
        assert_eq!(fs::read(root.join("draft")).unwrap(), b"draft stays");
        fs::remove_file(root.join("draft")).unwrap();

        // An absent pane on its own says nothing about its process boundary.
        fs::write(root.join("pane-absent"), b"").unwrap();
        assert_eq!(code(&stop_args), "resume-source-unknown");
        assert_eq!(
            export().stdout_json()["error"]["code"],
            "resume-source-unknown"
        );
        // Even a stopped broker and exact missing pane cannot substitute
        // for a persisted process boundary.
        let mut incomplete = record.clone();
        let identity = incomplete["delete_tmux_identity"].as_object_mut().unwrap();
        for field in [
            "process_group_id",
            "process_session_id",
            "process_session_members",
        ] {
            identity.remove(field);
        }
        write_private(&record_path, incomplete.to_string().as_bytes());
        fs::copy(
            root.join("stopped-registry"),
            coordination.join("registry.json"),
        )
        .unwrap();
        assert_eq!(code(&stop_args), "resume-source-unknown");
        assert_eq!(
            export().stdout_json()["error"]["code"],
            "resume-source-unknown"
        );
        write_private(&record_path, &record_bytes);
        write_private(
            &coordination.join("registry.json"),
            registry("ready").to_string().as_bytes(),
        );
        fs::remove_file(root.join("pane-absent")).unwrap();
    } else {
        fs::write(root.join("background"), b"").unwrap();
        assert_eq!(code(&stop_args), "resume-move-source-busy");
        fs::remove_file(root.join("background")).unwrap();
    }
    assert!(!root.join("input").exists());

    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TMPDIR", "/tmp")
        .env("HOME", &root)
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .env("AGENT_SESSION_CLAUDE_ACCOUNT_BROKER", r#"["/bin/false"]"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("--state-dir")
        .arg(&state)
        .arg("resume-move")
        .args(stop_args)
        .args(["--format", "json"]);
    fs::write(root.join("hold-exit"), b"").unwrap();
    let mut first_stop = Runtime::spawn(&mut command);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("exit-request").exists() {
        assert!(
            Instant::now() < deadline,
            "stop did not send a graceful exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(code(&stop_args), "resume-move-source-busy");
    assert_eq!(
        export().stdout_json()["error"]["code"],
        "resume-source-live"
    );
    fs::remove_file(root.join("hold-exit")).unwrap();
    let output = first_stop.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(runtime.0.take().unwrap().wait().unwrap().success());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        receipt["schema_version"],
        "cli.agent-session.resume-move-stop.v1"
    );
    assert_eq!(
        receipt["data"],
        json!({
            "outcome":"stopped", "session_id":"source", "session_incarnation":INCARNATION,
            "session_generation":1, "provider":agent, "provider_session_id":THREAD,
            "account":"fixture-account", "account_binding_revision":1
        })
    );
    let repeated = run(&stop_args).stdout_json();
    assert_eq!(
        repeated["error"]["code"],
        "resume-move-source-already-stopped"
    );
    assert_eq!(repeated["error"]["details"]["receipt"], receipt["data"]);
    let expected_input: &[u8] = if agent == "codex" {
        b"/quit\n"
    } else {
        b"/exit\n"
    };
    assert_eq!(fs::read(root.join("input")).unwrap(), expected_input);
    if replace_before_export {
        // Model the new record published by an explicit resume. The old
        // receipt and a valid old ticket still name the previous generation.
        record["runtime"]["launch_id"] = json!(THREAD);
        record["runtime"]["generation"] = json!(2);
        let binding = &mut record["codex_account_binding"];
        binding["applied_runtime_id"] = json!(THREAD);
        let replacement = record.to_string();
        write_private(&record_path, replacement.as_bytes());
        assert_eq!(code(&stop_args), "resume-move-session-mismatch");
        assert_eq!(
            export().stdout_json()["error"]["code"],
            "resume-move-session-mismatch",
        );
        assert!(
            !state
                .join("coordination/resume-move-source.v1.json")
                .exists()
        );
        assert_eq!(fs::read(&record_path).unwrap(), replacement.as_bytes());
        assert_eq!(fs::read(root.join("input")).unwrap(), expected_input);
        return;
    }
    let exported = export();
    assert_eq!(
        exported.code,
        0,
        "{} {}",
        exported.stdout_text(),
        exported.stderr_text()
    );
    let proof: SignedRecord =
        serde_json::from_value(exported.stdout_json()["data"].clone()).unwrap();
    let payload: serde_json::Value = serde_json::from_str(&proof.payload).unwrap();
    assert_eq!(payload["schema_version"], STOP_PROOF_SCHEMA);
    assert_eq!(payload["binding"]["source_incarnation"], INCARNATION);
    assert_eq!(payload["source_generation"], 1);
    assert_eq!(
        payload["history"]["sha256"],
        sha256(history_bytes.as_bytes())
    );
    assert_eq!(fs::read(&record_path).unwrap(), record_bytes);
    assert_eq!(fs::read(&history).unwrap(), history_bytes.into_bytes());
}

const TMUX: &str = r#"#!/bin/sh
set -eu
root=${0%/*}
pid=$(cat "$root/pane-pid")
agent=$(cat "$root/agent")
case "$1" in
  has-session)
    if [ -f "$root/pane-absent" ] || ! grep -qv ') Z ' "/proc/$pid/stat" 2>/dev/null; then
      printf "can't find session: fixture-source\n" >&2
      exit 1
    fi
    ;;
  display-message)
    printf '$7 %%8 %s\n' "$pid"
    ;;
  show-environment)
    case "$4" in
      AGENT_SESSION_ID) printf '%s=source\n' "$4" ;;
      AGENT_SESSION_STATE_DIR) printf '%s=%s/state\n' "$4" "$root" ;;
      AGENT_SESSION_RUNTIME_ID) printf '%s=00000000-0000-4000-8000-000000000032\n' "$4" ;;
      *) exit 2 ;;
    esac
    ;;
  capture-pane)
    if [ "$agent" = claude ]; then
      printf '────────────────────────────\n❯ \n────────────────────────────\n'
      if [ -f "$root/background" ]; then
        printf '  1 shell · status\n'
      else
        printf '  status\n'
      fi
    elif [ -f "$root/draft" ]; then
      printf '› %s\n\n  status\n' "$(cat "$root/draft")"
    else
      printf '› \033[2mAsk Codex to do anything\033[22m\n\n  status\n'
    fi
    ;;
  load-buffer) cp "$4" "$root/buffer" ;;
  paste-buffer)
    test -f "$root/buffer"
    for arg do target=$arg; done
    test "$target" = "%8"
    ;;
  send-keys)
    test "$3" = "%8"
    test "$4" = Enter
    input=$(cat "$root/buffer")
    if [ "$agent" = claude ]; then test "$input" = /exit; else test "$input" = /quit; fi
    printf '%s\n' "$input" >> "$root/input"
    touch "$root/exit-request"
    ;;
  *) exit 2 ;;
esac
"#;
