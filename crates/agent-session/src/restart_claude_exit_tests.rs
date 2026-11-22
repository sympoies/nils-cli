use super::*;
use pretty_assertions::{assert_eq, assert_ne};

const BACKGROUND_DIALOG: &str = "Background work is running
The following will stop when you exit:
  shell: synthetic job
  monitor: synthetic watch

❯ 1. Exit and stop tasks
  2. Move to background and exit
  3. Stay

Enter to select · Esc to cancel
";

const FEEDBACK_DIALOG: &str = "You have 1 unsent feedback draft
  Enter to review & send · Esc to discard and exit
";

struct RunningClaude {
    tmux: fixture::FakeTmux,
    context: CliContext,
    launch: String,
    root: tempfile::TempDir,
}

impl RunningClaude {
    fn new(dialog: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let context = context(root.path());
        seed_exited_session(&context, root.path());
        let tmux = fake_tmux(root.path());
        let launch = running_session(&context, &tmux);
        session_ready(&context, &launch);
        fs::write(root.path().join("exit-dialog"), dialog).unwrap();
        fs::copy(root.path().join("screen"), root.path().join("after-escape")).unwrap();
        Self {
            tmux,
            context,
            launch,
            root,
        }
    }

    fn restart(&self) -> Result<Receipt, CliError> {
        restart(&self.context, &restart_args(&self.tmux, false, 5))
    }

    fn input_log(&self) -> String {
        fs::read_to_string(self.root.path().join("tmux.log")).unwrap()
    }

    fn assert_still_running(&self) {
        let record = load_session_record(&self.context, ID).unwrap();
        assert_eq!(launch_id(&record).as_deref(), Some(self.launch.as_str()));
        assert_eq!(
            crate::session_status(&self.context, &self.tmux, &record),
            "running"
        );
        assert!(!super::super::fence_path(&self.context.state_dir, ID).exists());
    }
}

#[test]
fn background_work_in_claude_footer_refuses_before_exit() {
    for footer in [
        "1 shell, 1 monitor",
        "1 monitor",
        "2 background tasks",
        "\x1b[7m↳ 2 background\x1b[0m",
    ] {
        let fixture = RunningClaude::new(BACKGROUND_DIALOG);
        fs::write(
            fixture.root.path().join("screen"),
            format!("────────────────────\n❯\u{a0}\n────────────────────\n  ⏵⏵ accept edits on · {footer}\n"),
        )
        .unwrap();

        let error = fixture.restart().unwrap_err();

        assert_eq!(error.code(), "restart-session-not-idle");
        assert_eq!(details(&error)["cause"], "claude-background-work");
        let log = fixture.input_log();
        assert!(
            !log.contains("paste-buffer") && !log.contains("send-keys"),
            "{log}"
        );
        fixture.assert_still_running();
    }
}

#[test]
fn claude_background_exit_dialog_rolls_back_to_the_same_pane() {
    let fixture = RunningClaude::new(BACKGROUND_DIALOG);

    let error = fixture.restart().unwrap_err();

    assert_eq!(error.code(), "restart-exit-dialog");
    assert_eq!(details(&error)["cause"], "claude-background-work");
    assert_eq!(details(&error)["receipt"]["outcome"], "not-restarted");
    fixture.assert_still_running();
    let screen = fs::read_to_string(fixture.root.path().join("screen")).unwrap();
    assert_eq!(
        crate::send_submit::exact_composer_input("claude", &screen).as_deref(),
        Some("")
    );
    let log = fixture.input_log();
    assert_eq!(log.matches(" Enter\n").count(), 1, "{log}");
    assert_eq!(log.matches(" Escape\n").count(), 1, "{log}");
}

#[test]
fn claude_exit_dialog_that_stays_open_has_a_typed_outcome() {
    let fixture = RunningClaude::new(BACKGROUND_DIALOG);
    fs::write(fixture.root.path().join("after-escape"), BACKGROUND_DIALOG).unwrap();

    let error = fixture.restart().unwrap_err();

    assert_eq!(error.code(), "restart-exit-dialog-unresolved");
    assert_eq!(details(&error)["cause"], "claude-background-work");
    assert_eq!(
        details(&error)["receipt"]["outcome"],
        "exit-dialog-unresolved"
    );
    fixture.assert_still_running();
    let log = fixture.input_log();
    assert_eq!(log.matches(" Escape\n").count(), 1, "{log}");
}

#[test]
fn claude_feedback_exit_dialog_discards_and_restarts() {
    let fixture = RunningClaude::new(FEEDBACK_DIALOG);
    fs::write(fixture.root.path().join("escape-exits"), "").unwrap();
    let ready = on_relaunched_prompt(&fixture.context, fixture.root.path(), &fixture.launch);

    let receipt = fixture.restart().unwrap();

    ready.join().unwrap();
    assert_eq!(receipt.outcome, "ready");
    assert_eq!(receipt.stop_method, Some("provider-exit"));
    assert_eq!(receipt.provider_session_id.as_deref(), Some(CONVERSATION));
    assert_ne!(
        receipt.old.session_incarnation,
        receipt.new.session_incarnation
    );
    let log = fixture.input_log();
    assert_eq!(log.matches(" Enter\n").count(), 1, "{log}");
    assert_eq!(log.matches(" Escape\n").count(), 1, "{log}");
}
