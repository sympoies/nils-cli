use super::*;
use crate::coordination::broker::test_support;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;

const ID: &str = "restart-fixture";
const OLD: &str = "exited-incarnation";
const CONVERSATION: &str = "provider-conversation";

fn context(root: &Path) -> CliContext {
    CliContext {
        state_dir: root.join("state"),
        host: None,
    }
}

fn executable(path: &Path, body: &str) -> PathBuf {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    path.to_path_buf()
}

/// A tmux fake whose session exists only after `new-session`. Like the real
/// held launch, the new incarnation's heartbeat starts once the broker gate
/// opens, so the prior incarnation's beat is the one on disk while a resume
/// provisions coordination. An Enter key stands in for the provider exiting on
/// its exit command, and a `fail-launch` marker makes the next launch fail.
fn fake_tmux(root: &Path) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
root={root}
printf '%s\n' "$*" >> "$root/tmux.log"
stop_pane() {{ kill -- "-$(cat "$root/pane")" 2>/dev/null; rm -f "$root/running"; }}
if [ "$1" != new-session ] && [ ! -f "$root/running" ]; then
  printf '%s\n' "can't find session: fixture" >&2
  exit 1
fi
case "$1" in
  new-session)
    [ -f "$root/fail-launch" ] && exit 1
    heartbeat=''; slot=0; env=''
    : > "$root/env"
    for arg in "$@"; do
      if [ "$env" = 1 ]; then printf '%s\n' "$arg" >> "$root/env"; env=''; continue; fi
      [ "$arg" = -e ] && env=1
      if [ "$slot" = 2 ]; then incarnation="$arg"; slot=3
      elif [ "$slot" = 1 ]; then slot=2
      elif [ "$slot" = 0 ]; then case "$arg" in */coordination/heartbeat) heartbeat="$arg"; slot=1 ;; esac
      fi
    done
    if [ -n "$heartbeat" ]; then
      gate="$(dirname "$heartbeat")/broker-provisioned"
      (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done
       printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 &
    fi
    setsid sh -c 'while [ -d "$1" ]; do sleep 0.05; done' pane "$root" >/dev/null 2>&1 </dev/null &
    printf '%s\n' "$!" > "$root/pane"
    : > "$root/running"
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat "$root/pane")"
    ;;
  display-message) printf '%s\t%s\t%s\n' '$77' '%77' "$(cat "$root/pane")" ;;
  show-environment) grep "^$4=" "$root/env" || exit 1 ;;
  capture-pane) cat "$root/screen" ;;
  send-keys) case " $* " in *" Enter "*) stop_pane ;; esac ;;
  if-shell|kill-session) stop_pane ;;
esac
exit 0
"#,
        root = shell_words::quote(&root.to_string_lossy()),
    );
    // An idle Claude Code composer with nothing typed.
    fs::write(
        root.join("screen"),
        "────────────────────\n❯\u{a0}\n────────────────────\n  status\n",
    )
    .unwrap();
    executable(&root.join("tmux"), &script)
}

/// A Claude session whose runtime exited on its own: its pane process group is
/// gone and its launch wrapper's `broker stop` already revoked the broker, but
/// the last heartbeat is still inside the freshness window.
fn seed_exited_session(context: &CliContext, root: &Path) -> SessionRecord {
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let agent_bin = executable(
        &root.join("claude"),
        "#!/bin/sh\n[ \"$1\" = --version ] && printf '9.9.9 (Fixture)\\n'\n",
    );
    let group = test_support::exited_process_group();
    let mut identity = test_support::process_group_identity(group);
    identity["launch_id"] = json!(OLD);
    let record: SessionRecord = serde_json::from_value(json!({
        "schema_version": crate::SESSION_DOCUMENT_VERSION,
        "id": ID,
        "agent": "claude",
        "agent_bin": agent_bin,
        "mode": "interactive",
        "title": null,
        "cwd": cwd,
        "tmux_session": "as-restart-fixture",
        "prompt_file": null,
        "log_file": null,
        "created_at": "2030-01-01T00:00:00Z",
        "updated_at": "2030-01-01T00:00:00Z",
        "provider_resume": {
            "provider": "claude",
            "session_id": CONVERSATION,
            "captured_at": "2030-01-01T00:00:00Z",
            "capture_method": "fixture",
            "resume_args": ["--resume", CONVERSATION]
        },
        "runtime": {
            "kind": "tmux",
            "tmux_session": "as-restart-fixture",
            "generation": 1,
            "started_at": "2030-01-01T00:00:00Z",
            "launch_id": OLD
        },
        "delete_tmux_identity": identity.clone(),
        "agent_args": []
    }))
    .unwrap();
    crate::write_session_record(context, &record).unwrap();
    test_support::seed_live_broker(context, ID, OLD, identity);
    crate::coordination::revoke(context, &record).unwrap();
    record
}

fn claude_hook(context: &CliContext, launch: &str, payload: Value) {
    crate::activity::ingest_provider_hook_input(
        context,
        AgentKind::Claude,
        None,
        crate::activity::ProviderHookInput {
            id: ID,
            runtime_id: launch,
            payload: &serde_json::to_vec(&payload).unwrap(),
            attention_authority: None,
        },
    )
    .unwrap();
}

fn session_ready(context: &CliContext, launch: &str) {
    claude_hook(
        context,
        launch,
        json!({"hook_event_name": "SessionStart", "source": "resume", "session_id": CONVERSATION}),
    );
}

/// Resume the exited fixture into a running runtime and return its launch id.
fn running_session(context: &CliContext, tmux: &Path) -> String {
    crate::resume_session_by_id(context, ID, tmux).expect("resume fixture");
    launch_id(&load_session_record(context, ID).unwrap()).unwrap()
}

fn restart_args(tmux: &Path, force: bool, ready_timeout: u64) -> RestartArgs {
    RestartArgs {
        id: ID.to_string(),
        force,
        stop_timeout: 5,
        ready_timeout,
        tmux_bin: Some(tmux.to_path_buf()),
        format: nils_common::cli_contract::OutputFormat::Json,
    }
}

fn details(error: &CliError) -> Value {
    serde_json::to_value(&error.clone().into_inner().details).unwrap()
}

/// Right after a provider exits cleanly, `resume` was refused with "the prior
/// coordination incarnation is still live" until the exited incarnation's
/// last heartbeat aged out, although its runtime was verified stopped.
#[cfg(target_os = "linux")]
#[test]
fn resume_right_after_a_provider_exit_relaunches_the_stopped_incarnation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    assert!(coordination::broker::heartbeat_fresh(&context, ID, OLD, 0));
    let tmux = fake_tmux(tmp.path());

    let resumed = match crate::resume_session_by_id(&context, ID, &tmux) {
        Ok(view) => serde_json::to_value(view).unwrap(),
        Err(error) => panic!(
            "resume after exit failed: {error:?}\n{}",
            fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default()
        ),
    };
    assert_eq!(resumed["status"], "running");
    let record = load_session_record(&context, ID).unwrap();
    let runtime = record.runtime.as_ref().unwrap();
    assert_ne!(runtime.launch_id, OLD);
    assert_eq!(runtime.generation, 2);
    assert_eq!(
        record.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
}

#[cfg(target_os = "linux")]
#[test]
fn idle_restart_relaunches_in_place_and_reports_a_ready_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    // The relaunched provider reports its resumed prompt once it is up.
    let watcher = {
        let context = context.clone();
        let old_launch = old_launch.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                if let Some(launch) = load_session_record(&context, ID)
                    .ok()
                    .and_then(|record| launch_id(&record))
                    .filter(|launch| *launch != old_launch)
                {
                    session_ready(&context, &launch);
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
    };

    let receipt = restart(&context, &restart_args(&tmux, false, 30));
    watcher.join().unwrap();

    let receipt = match receipt {
        Ok(receipt) => serde_json::to_value(receipt).unwrap(),
        Err(error) => panic!(
            "restart failed: {error:?}\n{}",
            fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default()
        ),
    };
    let record = load_session_record(&context, ID).unwrap();
    let new_launch = launch_id(&record).unwrap();
    assert_ne!(new_launch, old_launch);
    assert_eq!(receipt["outcome"], "ready");
    assert_eq!(receipt["stop_method"], "provider-exit");
    assert_eq!(receipt["provider_session_id"], CONVERSATION);
    assert_eq!(receipt["tmux_session"], "as-restart-fixture");
    assert_eq!(receipt["old"]["session_incarnation"], old_launch.as_str());
    assert_eq!(receipt["old"]["generation"], 2);
    assert_eq!(receipt["new"]["session_incarnation"], new_launch.as_str());
    assert_eq!(receipt["new"]["generation"], 3);
    assert_eq!(receipt["new"]["provider_version"], "9.9.9 (Fixture)");
    assert!(receipt["new"]["pane_pid"].is_i64());
    assert!(receipt["ready_at"].is_string());
    assert_eq!(
        record.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
    let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap();
    // The exit command was typed and the provider left on its own.
    assert!(
        log.contains("paste-buffer") && !log.contains("if-shell"),
        "{log}"
    );
    // The relaunched incarnation owns a ready coordination broker again, so
    // mailbox delivery reaches it.
    let broker = coordination::broker::status(
        &context,
        crate::cli::BrokerStatusArgs {
            session: ID.to_string(),
            capability_file: None,
            authenticated: false,
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(broker["session_incarnation"], new_launch.as_str());
    assert_eq!(broker["state"], "ready");
}

#[cfg(target_os = "linux")]
#[test]
fn busy_session_is_refused_without_force_and_left_running() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name": "UserPromptSubmit", "session_id": CONVERSATION, "prompt": "work"}),
    );

    let error = restart(&context, &restart_args(&tmux, false, 1)).unwrap_err();

    assert_eq!(error.code(), "restart-session-not-idle");
    assert_eq!(details(&error)["cause"], "send-session-not-idle");
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&record).as_deref(), Some(launch.as_str()));
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
    let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap();
    assert!(
        !log.contains("send-keys") && !log.contains("if-shell"),
        "{log}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn failed_relaunch_leaves_a_receipt_and_a_resumable_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    fs::write(tmp.path().join("fail-launch"), "").unwrap();

    let error = restart(&context, &restart_args(&tmux, false, 1)).unwrap_err();

    assert_eq!(
        error.code(),
        "restart-relaunch-failed",
        "{error:?}\n{}",
        fs::read_to_string(tmp.path().join("tmux.log")).unwrap()
    );
    let details = details(&error);
    let recovery = format!("agent-session resume {ID}");
    assert_eq!(details["recovery"], recovery.as_str());
    let receipt = &details["receipt"];
    assert_eq!(receipt["outcome"], "relaunch-failed");
    assert_eq!(receipt["stop_method"], "provider-exit");
    assert_eq!(receipt["old"]["session_incarnation"], old_launch.as_str());
    assert!(receipt["stopped_at"].is_string());
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(crate::session_status(&context, &tmux, &record), "stopped");

    // The printed recovery command relaunches the same conversation.
    fs::remove_file(tmp.path().join("fail-launch")).unwrap();
    crate::resume_session_by_id(&context, ID, &tmux).expect("recovery resume");
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
    assert_ne!(launch_id(&record).as_deref(), Some(old_launch.as_str()));
    assert_eq!(
        record.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
}
