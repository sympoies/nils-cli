//! Submission outcomes when the pane disappears after an accepted Enter.
use super::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

fn confirms_exit(guards: &[&str], pending_pane: bool) {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("exit-after-enter"), "").unwrap();
    if pending_pane {
        fs::write(fixture.root().join("exit-pending-pane"), "").unwrap();
    }
    let output = fixture.send_text(Some("/exit"), guards);
    let value = output.stdout_json();
    assert_eq!(output.code, 0, "{value}");
    assert_eq!(
        value["data"]["submission"]["outcome"], "submitted",
        "{value}"
    );
    assert_eq!(value["data"]["submission"]["enter_presses"], 1, "{value}");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter"))
            .count(),
        1
    );
}

#[test]
fn send_confirms_submission_when_the_runtime_exits_after_enter() {
    confirms_exit(&[], false);
}

#[test]
fn guarded_send_confirms_submission_when_the_runtime_exits_after_enter() {
    confirms_exit(&["--require-empty-composer"], false);
}

#[test]
fn an_unknown_runtime_probe_does_not_confirm_submission() {
    for guards in [vec![], vec!["--require-empty-composer"]] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("exit-after-enter"), "").unwrap();
        fs::write(fixture.root().join("probe-unknown-after-enter"), "").unwrap();
        let output = fixture.send_text(Some("/exit"), &guards);
        if guards.is_empty() {
            assert_eq!(
                output.stdout_json()["data"]["submission"]["outcome"],
                "unverified"
            );
        } else {
            fixture.refused(&output, "send-submit-unconfirmed");
        }
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
    }
}

#[test]
fn a_stale_pending_pane_does_not_turn_runtime_exit_into_a_stuck_submission() {
    confirms_exit(&[], true);
    confirms_exit(&["--require-empty-composer"], true);
}
