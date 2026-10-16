//! Soft-wrap fidelity and truthful post-paste error receipts.
use super::{CmdOptions, Fixture, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use std::fs;

fn templates() -> Vec<String> {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../src/send_submit/composer/claude-statusline-captures.json"
    ))
    .unwrap();
    data["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["template"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn statusline_soft_wrapped_own_text_submits_once_with_exact_spaces() {
    for template in templates() {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("statusline-template"), template).unwrap();
        fs::write(fixture.root().join("soft-wrap"), "").unwrap();
        let text =
            "Continue the generic fixture investigation and preserve every occupied space.  ";
        let output = fixture.send_text(Some(text), &["--require-idle", "--require-empty-composer"]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
        let actual: Vec<String> = submissions
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(actual, vec![text]);
    }
}

#[test]
fn statusline_hard_newlines_refuse_after_paste_without_enter() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("statusline-template"), &templates()[0]).unwrap();
    let text = "first line\n  second line";
    let output = fixture.send_text(Some(text), &["--require-empty-composer"]);
    fixture.refused(&output, "send-composer-unobservable");
    assert_eq!(output.stdout_json()["error"]["details"]["pasted"], true);
    assert!(!fixture.root().join("submissions").exists());
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        text
    );
}

#[test]
fn statusline_post_paste_io_and_record_failures_report_pasted() {
    for marker in ["key-fails", "corrupt-record-after-paste"] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("statusline-template"), &templates()[0]).unwrap();
        fs::write(fixture.root().join(marker), "").unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        assert_ne!(output.code, 0);
        assert_eq!(
            output.stdout_json()["error"]["details"]["pasted"],
            true,
            "marker {marker}"
        );
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "continue-fixture"
        );
    }
}

#[test]
fn statusline_text_only_send_enforces_empty_composer_before_paste() {
    for draft in ["user draft", " ", "first line\n  second line"] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("statusline-template"), &templates()[0]).unwrap();
        fs::write(fixture.root().join("composer"), draft).unwrap();
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                fixture.root().join("state").to_str().unwrap(),
                "send",
                "guarded",
                "--tmux-bin",
                fixture.root().join("tmux").to_str().unwrap(),
                "--format",
                "json",
                "--require-empty-composer",
                "--text",
                "x",
            ],
            &CmdOptions::new()
                .with_cwd(fixture.root())
                .with_env_remove_prefix("AGENT_SESSION_"),
        );
        fixture.refused(
            &output,
            if draft.contains('\n') {
                "send-composer-unobservable"
            } else {
                "send-composer-not-empty"
            },
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            draft
        );
    }
}
