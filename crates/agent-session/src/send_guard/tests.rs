use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[test]
fn malformed_composer_and_expired_budget_report_distinct_stages() {
    let temp = tempfile::tempdir().unwrap();
    let tmux = temp.path().join("tmux");
    fs::write(
        &tmux,
        "#!/bin/sh\nprintf '\\033]52;;private-fixture-marker\\007'\n",
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let record: SessionRecord = serde_json::from_value(json!({
        "schema_version": "agent-session.session.v1", "id": "fixture", "agent": "claude",
        "mode": "interactive", "title": null, "cwd": "/", "tmux_session": "fixture",
        "prompt_file": null, "log_file": null, "created_at": "", "updated_at": ""
    }))
    .unwrap();
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    let args = GuardArgs {
        require_empty_composer: true,
        ..GuardArgs::default()
    };
    let admission = Admission {
        stopped_idle: &Default::default(),
        context: &context,
        record: &record,
        args: &args,
        tmux_bin: &tmux,
        verify_composer: true,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    TEST_RETRY_WINDOW.set(Duration::ZERO);
    for (stage, pasted) in [("admission", None), ("submit", Some("read mailbox"))] {
        let error = admission
            .observe_composer("fixture:0.0", pasted, stage, false, false)
            .unwrap_err();
        assert_eq!(error.code(), "send-composer-unobservable");
        let details = error.details().unwrap();
        assert_eq!(details["stage"], stage);
        assert_eq!(details["observation"], "pane-unparsable");
        assert_eq!(details["pasted"], stage == "submit");
        assert!(!error.message().contains("private-fixture-marker"));
    }
    TEST_RETRY_WINDOW.set(POST_PASTE_OBSERVATION_RETRY_WINDOW);
    let expired = Admission {
        deadline: Instant::now() - Duration::from_secs(1),
        ..admission
    };
    let error = expired
        .observe_composer("fixture:0.0", Some("read mailbox"), "submit", false, false)
        .unwrap_err();
    assert_eq!(error.code(), "send-guard-observation-timeout");
    assert_eq!(error.details().unwrap()["stage"], "submit");
    assert_eq!(error.details().unwrap()["guard"], "observation");
}

mod stopped_idle;

mod transaction;
