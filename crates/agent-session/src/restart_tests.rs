use super::*;
use crate::coordination::broker::test_support;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[path = "auth_recovery_tests.rs"]
mod auth_recovery_tests;

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
/// its exit command unless `hold-exit` exists, a `fail-launch` marker makes the
/// next launch fail, a `stall` marker stalls pane identity observations and a
/// `stall-status` marker stalls `has-session`.
fn fake_tmux(root: &Path) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
root={root}
printf '%s\n' "$*" >> "$root/tmux.log"
stop_pane() {{ kill -- "-$(cat "$root/pane")" 2>/dev/null; rm -f "$root/running"; }}
if [ -f "$root/exit-on-probe" ] && [ "$1" = display-message ]; then
  stop_pane
  rm -f "$root/exit-on-probe"
fi
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
  has-session) [ -f "$root/stall-status" ] && {{ : > "$root/status-stalled"; sleep 20; }} ;;
  show-environment) [ -f "$root/stall" ] && sleep 20; grep "^$4=" "$root/env" || exit 1 ;;
  capture-pane) cat "$root/screen" ;;
  send-keys) case " $* " in *" Enter "*) [ -f "$root/hold-exit" ] || stop_pane ;; esac ;;
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
        "origin": {"parent":{"session_id":"origin-parent","machine":"worker-host"}},
        "workspace": "example-workspace",
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

/// Fail the next post-launch persistence of restarts in `context` only.
fn arm_post_launch_failure(context: &CliContext) {
    fs::create_dir_all(&context.state_dir).unwrap();
    fs::write(context.state_dir.join(FAIL_POST_LAUNCH), "").unwrap();
}

fn details(error: &CliError) -> Value {
    serde_json::to_value(&error.clone().into_inner().details).unwrap()
}

/// Right after a provider exits cleanly, `resume` was refused with "the prior
/// coordination incarnation is still live" until the exited incarnation's
/// last heartbeat aged out, although its runtime was verified stopped.
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

#[test]
fn idle_restart_relaunches_in_place_and_reports_a_ready_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    let origin = crate::origin::Fields::of(&seed_exited_session(&context, tmp.path()));
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    let watcher = on_relaunched_prompt(&context, tmp.path(), &old_launch);

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
    assert_eq!(crate::origin::Fields::of(&record), origin);
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

fn log(root: &Path) -> String {
    fs::read_to_string(root.join("tmux.log")).unwrap_or_default()
}

/// Run `action` once the relaunch replaced `old_launch`.
fn on_relaunch(
    context: &CliContext,
    old_launch: &str,
    action: impl FnOnce(&CliContext, &str) + Send + 'static,
) -> thread::JoinHandle<()> {
    let context = context.clone();
    let old_launch = old_launch.to_string();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(launch) = load_session_record(&context, ID)
                .ok()
                .and_then(|record| launch_id(&record))
                .filter(|launch| *launch != old_launch)
            {
                action(&context, &launch);
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    })
}

/// Like the provider, report the resumed prompt only once the relaunched pane
/// runs, after resume activated the new runtime.
fn on_relaunched_prompt(
    context: &CliContext,
    root: &Path,
    old_launch: &str,
) -> thread::JoinHandle<()> {
    let root = root.to_path_buf();
    on_relaunch(context, old_launch, move |context, launch| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while log(&root).matches("new-session").count() < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        session_ready(context, launch);
    })
}

/// Queue one mailbox notification for the running fixture incarnation.
fn queued_notification(
    context: &CliContext,
    root: &Path,
) -> crate::coordination::NotificationCandidate {
    let env = fs::read_to_string(root.join("env")).unwrap();
    let capability = env
        .lines()
        .find_map(|line| line.strip_prefix("AGENT_SESSION_CAPABILITY_FILE="))
        .unwrap();
    let body = root.join("body.md");
    fs::write(&body, "status?").unwrap();
    crate::coordination::mailbox::send(
        context,
        crate::cli::MessageSendArgs {
            from_session: ID.to_string(),
            to_session: ID.to_string(),
            to_machine: None,
            body_file: body,
            capability_file: Some(PathBuf::from(capability)),
            idempotency_key: "restart-fixture-note".to_string(),
            reply_to: None,
            expires_in: None,
            format: nils_common::cli_contract::OutputFormat::Json,
            category: None,
        },
    )
    .expect("queue a notification");
    coordination::pending_notifications(context)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.target_session_id == ID)
        .expect("queued notification")
}

fn try_notification(
    context: &CliContext,
    candidate: &crate::coordination::NotificationCandidate,
) -> bool {
    let mut guard = coordination::lock_session_quiescence(
        context,
        &candidate.target_session_id,
        &candidate.target_incarnation,
    )
    .unwrap();
    guard.begin_notification_attempt(candidate).unwrap()
}

/// An exact notification submission already in progress must refuse the
/// restart before the exit command is typed, not only at the later stop.
#[test]
fn in_progress_notification_refuses_before_the_exit_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    let candidate = queued_notification(&context, tmp.path());
    assert!(try_notification(&context, &candidate));

    let error = restart(&context, &restart_args(&tmux, false, 1)).unwrap_err();

    assert_eq!(
        error.code(),
        "coordination-notification-submission-in-progress"
    );
    assert!(
        !log(tmp.path()).contains("paste-buffer"),
        "{}",
        log(tmp.path())
    );
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
}

/// Between the exit command and the stop, new notification attempts and
/// terminal input are fenced; a provider that does not leave on its own is
/// left running without --force, and the fence is released afterwards.
#[test]
fn exit_wait_fences_input_and_never_kills_without_force() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    let candidate = queued_notification(&context, tmp.path());
    fs::write(tmp.path().join("hold-exit"), "").unwrap();
    let racer = {
        let context = context.clone();
        let root = tmp.path().to_path_buf();
        let tmux = tmux.clone();
        let candidate = candidate.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !log(&root).contains("send-keys") && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let notified = try_notification(&context, &candidate);
            let sent = crate::send::send_to_session(
                &context,
                crate::cli::SendArgs {
                    id: ID.to_string(),
                    text: Some("new work".to_string()),
                    text_stdin: false,
                    keys: vec![SpecialKey::Enter],
                    allow_blocked: false,
                    guards: GuardArgs::default(),
                    tmux_bin: Some(tmux),
                    format: nils_common::cli_contract::OutputFormat::Json,
                },
            );
            (notified, sent.err().map(|error| error.code().to_string()))
        })
    };

    let mut args = restart_args(&tmux, false, 1);
    args.stop_timeout = 2;
    let error = restart(&context, &args).unwrap_err();
    let (notified, sent) = racer.join().unwrap();

    assert!(!notified, "a notification attempt raced the exit command");
    assert_eq!(sent.as_deref(), Some("session-restarting"));
    assert_eq!(error.code(), "restart-exit-timeout");
    assert_eq!(details(&error)["receipt"]["outcome"], "exit-timeout");
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&record).as_deref(), Some(launch.as_str()));
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
    assert!(!log(tmp.path()).contains("if-shell"), "{}", log(tmp.path()));
    // The fence ends with the restart.
    assert!(try_notification(&context, &candidate));
}

/// A resume that replaces the runtime while the restart waits for the exit is
/// kept, and the restart reports it with its receipt.
#[test]
fn runtime_replaced_during_the_exit_wait_is_reported_and_kept() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    fs::write(tmp.path().join("hold-exit"), "").unwrap();
    let replacer = {
        let context = context.clone();
        let root = tmp.path().to_path_buf();
        let tmux = tmux.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !log(&root).contains("send-keys") && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            // Another operator holds the session while the provider leaves,
            // its wrapper stops the broker, and the operator resumes it.
            let _lock = crate::acquire_session_record_lock(&context, ID).unwrap();
            let pane = fs::read_to_string(root.join("pane")).unwrap();
            let _ = ProcessCommand::new("kill")
                .arg("--")
                .arg(format!("-{}", pane.trim()))
                .status();
            fs::remove_file(root.join("running")).unwrap();
            fs::remove_file(root.join("hold-exit")).unwrap();
            let record = load_session_record(&context, ID).unwrap();
            coordination::revoke(&context, &record).unwrap();
            crate::resume_session_locked(&context, record, &tmux).expect("concurrent resume");
            launch_id(&load_session_record(&context, ID).unwrap()).unwrap()
        })
    };

    let error = restart(&context, &restart_args(&tmux, false, 1)).unwrap_err();
    let replacement = replacer.join().unwrap();

    assert_eq!(error.code(), "restart-superseded");
    let details = details(&error);
    assert_eq!(details["receipt"]["outcome"], "superseded");
    assert_eq!(
        details["recovery"],
        format!("agent-session glance {ID}").as_str()
    );
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&record).as_deref(), Some(replacement.as_str()));
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
}

/// Every readiness observation is capped by --ready-timeout, even one that
/// stalls, and an unready relaunch reports how to inspect it.
#[test]
fn ready_timeout_bounds_a_stalled_readiness_observation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    let root = tmp.path().to_path_buf();
    let staller = on_relaunch(&context, &old_launch, move |_, _| {
        fs::write(root.join("stall"), "").unwrap();
    });

    let error = restart(&context, &restart_args(&tmux, false, 2)).unwrap_err();
    staller.join().unwrap();

    assert_eq!(error.code(), "restart-not-ready");
    let details = details(&error);
    let receipt = &details["receipt"];
    assert_eq!(receipt["outcome"], "not-ready");
    assert_eq!(
        details["recovery"],
        format!("agent-session glance {ID}").as_str()
    );
    let relaunched: Timestamp = receipt["relaunched_at"].as_str().unwrap().parse().unwrap();
    let finished: Timestamp = receipt["finished_at"].as_str().unwrap().parse().unwrap();
    let waited = finished.duration_since(relaunched).as_secs_f64();
    // The stalled observation outlasts the ceiling, which sits far above the
    // 2 s budget so that CPU starvation cannot trip it.
    assert!(waited < 8.0, "readiness waited {waited}s past a 2s timeout");
}

#[test]
fn relaunched_provider_exit_reports_the_resume_recovery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    let root = tmp.path().to_path_buf();
    let crasher = on_relaunch(&context, &old_launch, move |_, _| {
        // Crash the relaunched provider once its runtime exists.
        let deadline = Instant::now() + Duration::from_secs(10);
        while log(&root).matches("new-session").count() < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(300));
        let _ = fs::remove_file(root.join("running"));
    });

    let error = restart(&context, &restart_args(&tmux, false, 5)).unwrap_err();
    crasher.join().unwrap();

    assert_eq!(error.code(), "restart-not-ready");
    let details = details(&error);
    assert_eq!(details["receipt"]["outcome"], "relaunch-exited");
    assert_eq!(
        details["recovery"],
        format!("agent-session resume {ID}").as_str()
    );
}

/// A persistence failure after the relaunch still returns the receipt with
/// the new incarnation and a command to inspect the running session.
#[test]
fn post_launch_persistence_failure_keeps_the_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    arm_post_launch_failure(&context);

    let error = restart(&context, &restart_args(&tmux, false, 1)).unwrap_err();

    assert_eq!(error.code(), "restart-post-launch-failed");
    let details = details(&error);
    let record = load_session_record(&context, ID).unwrap();
    let launch = launch_id(&record).unwrap();
    assert_ne!(launch, old_launch);
    assert_eq!(
        details["receipt"]["new"]["session_incarnation"],
        launch.as_str()
    );
    assert_eq!(
        details["recovery"],
        format!("agent-session glance {ID}").as_str()
    );
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
}

/// --force overrides only idle quiescence: a pane whose runtime identity does
/// not match the session is refused and left running, never terminated.
#[test]
fn force_never_overrides_a_runtime_identity_mismatch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    let env = tmp.path().join("env");
    let foreign = fs::read_to_string(&env).unwrap().replace(
        &format!("AGENT_SESSION_RUNTIME_ID={launch}"),
        "AGENT_SESSION_RUNTIME_ID=foreign",
    );
    fs::write(&env, foreign).unwrap();

    let error = restart(&context, &restart_args(&tmux, true, 1)).unwrap_err();

    assert_eq!(error.code(), "restart-identity-unverified");
    assert_eq!(details(&error)["cause"], "send-identity-mismatch");
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&record).as_deref(), Some(launch.as_str()));
    assert_eq!(crate::session_status(&context, &tmux, &record), "running");
    assert!(!log(tmp.path()).contains("if-shell"), "{}", log(tmp.path()));
}

/// The readiness status probe is capped by --ready-timeout as well: a stalled
/// tmux has-session must not hold the restart for the fixed probe timeout.
#[test]
fn ready_timeout_bounds_a_stalled_status_probe() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    let root = tmp.path().to_path_buf();
    // Stall has-session once the relaunch is recorded and readiness begins.
    let staller = on_relaunch(&context, &old_launch, move |context, launch| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline
            && !load_session_record(context, ID)
                .ok()
                .and_then(|record| record.extra.get(PROVIDER_VERSION_KEY).cloned())
                .is_some_and(|recorded| recorded["launch_id"] == launch)
        {
            thread::sleep(Duration::from_millis(5));
        }
        fs::write(root.join("stall-status"), "").unwrap();
    });

    let error = restart(&context, &restart_args(&tmux, false, 2)).unwrap_err();
    staller.join().unwrap();

    assert_eq!(error.code(), "restart-not-ready");
    assert!(
        tmp.path().join("status-stalled").exists(),
        "{}",
        log(tmp.path())
    );
    let receipt = &details(&error)["receipt"];
    let relaunched: Timestamp = receipt["relaunched_at"].as_str().unwrap().parse().unwrap();
    let finished: Timestamp = receipt["finished_at"].as_str().unwrap().parse().unwrap();
    let waited = finished.duration_since(relaunched).as_secs_f64();
    // An uncapped probe would wait out its 10 s command timeout.
    assert!(waited < 8.0, "readiness waited {waited}s past a 2s timeout");
}

/// A lifecycle writer that holds the session record past the stop timeout
/// after the exit leaves the restart with its receipt and a recovery command.
#[test]
fn post_exit_record_lock_contention_returns_the_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    fs::write(tmp.path().join("hold-exit"), "").unwrap();
    let restart_returned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let holder = {
        let context = context.clone();
        let root = tmp.path().to_path_buf();
        let restart_returned = restart_returned.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !log(&root).contains("send-keys") && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let _lock = crate::acquire_session_record_lock(&context, ID).unwrap();
            // The provider leaves while the record stays held.
            let pane = fs::read_to_string(root.join("pane")).unwrap();
            let _ = ProcessCommand::new("kill")
                .arg("--")
                .arg(format!("-{}", pane.trim()))
                .status();
            fs::remove_file(root.join("running")).unwrap();
            // Hold the record until the restart gives up, however long that takes.
            let deadline = Instant::now() + Duration::from_secs(30);
            while !restart_returned.load(std::sync::atomic::Ordering::SeqCst)
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let mut args = restart_args(&tmux, false, 1);
    args.stop_timeout = 1;
    let error = restart(&context, &args).unwrap_err();
    restart_returned.store(true, std::sync::atomic::Ordering::SeqCst);
    holder.join().unwrap();

    assert_eq!(error.code(), "restart-stop-failed", "{error:?}");
    let details = details(&error);
    assert_eq!(details["cause"], "session-record-lock-timeout");
    assert_eq!(details["receipt"]["outcome"], "stop-failed");
    assert_eq!(
        details["receipt"]["old"]["session_incarnation"],
        launch.as_str()
    );
    assert_eq!(
        details["recovery"],
        format!("agent-session glance {ID}").as_str()
    );
}

/// A second restart that overlaps the first is refused and leaves the first
/// restart's fence in place, so input stays fenced until the first ends.
#[test]
fn overlapping_restart_is_refused_and_keeps_the_first_fence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    let candidate = queued_notification(&context, tmp.path());
    fs::write(tmp.path().join("hold-exit"), "").unwrap();
    let first = {
        let context = context.clone();
        let mut args = restart_args(&tmux, false, 1);
        args.stop_timeout = 3;
        thread::spawn(move || restart(&context, &args).map(|_| ()))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !log(tmp.path()).contains("send-keys") && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }

    let mut args = restart_args(&tmux, false, 1);
    args.stop_timeout = 1;
    let second = restart(&context, &args).unwrap_err();

    assert_eq!(second.code(), "restart-in-progress");
    assert!(!try_notification(&context, &candidate));
    let sent = crate::send::send_to_session(
        &context,
        crate::cli::SendArgs {
            id: ID.to_string(),
            text: Some("new work".to_string()),
            text_stdin: false,
            keys: vec![SpecialKey::Enter],
            allow_blocked: false,
            guards: GuardArgs::default(),
            tmux_bin: Some(tmux.clone()),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    );
    assert_eq!(sent.unwrap_err().code(), "session-restarting");
    let first = first.join().unwrap().unwrap_err();
    assert_eq!(first.code(), "restart-exit-timeout");
    assert_eq!(log(tmp.path()).matches("paste-buffer").count(), 1);
    assert!(try_notification(&context, &candidate));
}

/// The post-launch failpoint is scoped to its state directory, so a restart
/// elsewhere in the same test process is unaffected.
#[test]
fn post_launch_failpoint_is_scoped_to_its_state_dir() {
    let armed = tempfile::TempDir::new().unwrap();
    arm_post_launch_failure(&context(armed.path()));
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let old_launch = running_session(&context, &tmux);
    session_ready(&context, &old_launch);
    let watcher = on_relaunched_prompt(&context, tmp.path(), &old_launch);

    let receipt = restart(&context, &restart_args(&tmux, false, 30));
    watcher.join().unwrap();

    assert_eq!(receipt.map(|receipt| receipt.outcome).ok(), Some("ready"));
}
