use super::*;
use pretty_assertions::assert_eq;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn fixture(root: &Path) -> (CliContext, SessionRecord) {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let record = serde_json::from_value(json!({
        "schema_version": crate::SESSION_DOCUMENT_VERSION, "id":"unexpected-exit", "agent":"claude",
        "mode":"interactive", "cwd":"/workspace", "tmux_session":"fixture",
        "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
        "runtime":{"kind":"tmux","tmux_session":"fixture","generation":1,"started_at":"2030-01-01T00:00:00Z","launch_id":"runtime-one"},
        "provider_resume":{"provider":"claude","session_id":"conversation-one","captured_at":"2030-01-01T00:00:00Z","capture_method":"fixture","resume_args":["--resume","conversation-one"]},
        "delete_tmux_identity":{"launch_id":"runtime-one","session_id":"$7","pane_id":"%7","pane_pid":7,"process_group_id":7}
    })).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    (context, record)
}

fn view(context: &CliContext, record: &SessionRecord, status: &str) -> Value {
    serde_json::to_value(crate::session_view(
        context,
        record,
        Some(status.into()),
        None,
    ))
    .unwrap()
}

struct Runtime(Child);
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

#[test]
fn sigint_exit_retains_reason_and_explicit_resume_without_restart_loop() {
    use std::os::unix::process::CommandExt;
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    let (context, record) = fixture(root.path());
    let ready = root.path().join("provider-ready");
    let launches = root.path().join("provider-launches");
    let gate = root.path().join("gate");
    let broker_gate = root.path().join("broker-gate");
    fs::write(&gate, "").unwrap();
    fs::write(&broker_gate, "").unwrap();
    let provider = r#"import os, signal, sys
signal.signal(signal.SIGINT, lambda *_: sys.exit(130))
with open(sys.argv[2], 'a') as f: f.write('launch\n')
with open(sys.argv[1], 'w') as f: f.write(str(os.getpid()))
signal.pause()
"#;
    let mut runtime = Runtime(
        Command::new("sh")
            .args(["-c", include_str!("../startup/held-launch.sh"), "fixture"])
            .arg(&gate)
            .arg(&broker_gate)
            .arg(root.path().join("heartbeat"))
            .arg(root.path().join("capability"))
            .args(["runtime-one", "1", "true"])
            .args(["python3", "-c", provider])
            .arg(&ready)
            .arg(&launches)
            .env("AGENT_SESSION_STATE_DIR", &context.state_dir)
            .env("AGENT_SESSION_ID", &record.id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "fake provider never became ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: i32 = fs::read_to_string(&ready).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    loop {
        if let Some(status) = runtime.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(130));
            break;
        }
        assert!(Instant::now() < deadline, "launcher did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..3 {
        observe_stopped(&context, &record);
        let projected = view(&context, &record, "stopped");
        assert_eq!(projected["runtime_exit"]["code"], 130);
        assert_eq!(projected["runtime_exit"]["reason"], "provider-exited");
        assert_eq!(projected["runtime_exit"]["unexpected"], true);
        assert_eq!(
            projected["runtime_exit"]["recovery"]["command"],
            "agent-session resume unexpected-exit"
        );
        assert_eq!(projected["resumable"], true);
        let board = crate::board::project_record(&projected, "fixture", None, false).unwrap();
        assert_eq!(board["runtime_exit"]["code"], 130);
        let session = crate::session_view(&context, &record, Some("stopped".into()), None);
        let text = super::recovery::list_line(&session);
        assert!(text.contains("130") && text.contains("agent-session resume unexpected-exit"));
        assert_eq!(
            crate::render_command_text(&session),
            format!("{}\n", session.attach_command)
        );
    }
    assert_eq!(fs::read_to_string(launches).unwrap(), "launch\n");
    assert_eq!(
        read(&context, &record.id, 20).unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        persisted.provider_resume.unwrap().session_id,
        "conversation-one"
    );
}

#[test]
fn explicit_stop_is_not_unexpected_and_old_exit_does_not_follow_resume() {
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    let (context, mut record) = fixture(root.path());
    super::record(
        &context,
        Some(&record),
        &record.id,
        "stop",
        "cli",
        Ok(()),
        None,
    );
    observe_stopped(&context, &record);
    let projected = view(&context, &record, "stopped");
    assert_eq!(projected["runtime_exit"]["unexpected"], false);
    assert!(projected["runtime_exit"]["recovery"].is_null());
    record.runtime.as_mut().unwrap().generation += 1;
    record.runtime.as_mut().unwrap().launch_id = "runtime-two".into();
    assert!(view(&context, &record, "running")["runtime_exit"].is_null());
    assert!(view(&context, &record, "stopped")["runtime_exit"].is_null());
}

#[test]
fn process_status_takes_precedence_over_prior_control_close() {
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    for code in [None, Some(1)] {
        let directory = root
            .path()
            .join(if code.is_some() { "known" } else { "unknown" });
        let (context, target) = fixture(&directory);
        super::recovery::record_control_close(&context, &target, "private-provider-detail");
        assert!(
            !serde_json::to_string(&read(&context, &target.id, 20).unwrap())
                .unwrap()
                .contains("private-provider-detail")
        );
        super::record(
            &context,
            Some(&target),
            &target.id,
            "exit-observed",
            "controller",
            Ok(()),
            Some(json!({"code":code, "signal":null, "reason":"runtime-exited", "stopped_by":null})),
        );
        let projected = view(&context, &target, "stopped");
        assert_eq!(
            projected["runtime_exit"]["reason"],
            if code.is_some() {
                "provider-exited"
            } else {
                "app-server-close"
            }
        );
        assert_eq!(projected["runtime_exit"]["code"], json!(code));
        assert_eq!(projected["runtime_exit"]["unexpected"], true);
        // An explicit stop can finish after the launcher's exit hook.
        super::record(
            &context,
            Some(&target),
            &target.id,
            "stop",
            "serve",
            Ok(()),
            None,
        );
        let projected = view(&context, &target, "stopped");
        assert_eq!(projected["runtime_exit"]["unexpected"], false);
        assert!(projected["runtime_exit"]["recovery"].is_null());
    }
}
