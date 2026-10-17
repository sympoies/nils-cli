//! Live Claude layout: crowded title border, written renderer padding, and the
//! cleanup of text a guarded send pasted before it refused.
use std::fs;

use pretty_assertions::assert_eq;
use serde_json::Value;

use super::Fixture;

fn live(fixture: &Fixture, markers: &[&str]) {
    fs::write(fixture.root().join("live-layout"), "").unwrap();
    for marker in markers {
        fs::write(fixture.root().join(marker), "").unwrap();
    }
}

fn guarded(fixture: &Fixture) -> super::CmdOutput {
    fixture.send(&[
        "--require-idle",
        "--require-empty-composer",
        "--idempotency-key",
        "live",
    ])
}

fn submissions(fixture: &Fixture) -> Vec<String> {
    fs::read_to_string(fixture.root().join("submissions"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn composer(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.root().join("composer")).unwrap()
}

fn backspaces(fixture: &Fixture) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|call| call[0] == "send-keys" && call[call.len() - 1] == "BSpace")
        .count()
}

fn details(output: &super::CmdOutput, code: &str) -> Value {
    assert_eq!(output.code, 1, "{}", output.stderr_text());
    let value = output.stdout_json();
    assert_eq!(value["error"]["code"], code, "{value}");
    value["error"]["details"].clone()
}

#[test]
fn padded_empty_composer_admits_and_submits() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer"]);
    let output = guarded(&fixture);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(submissions(&fixture), ["\"continue-fixture\""]);
}

#[test]
fn padded_suggestion_composer_admits_and_submits() {
    for markers in [
        &["suggestion-words"][..],
        &["suggestion-words", "long-title"],
    ] {
        let fixture = Fixture::new("claude");
        live(&fixture, markers);
        fs::write(
            fixture.root().join("suggestion"),
            "Read your mailbox and proceed",
        )
        .unwrap();
        let output = guarded(&fixture);
        assert_eq!(output.code, 0, "{markers:?}: {}", output.stdout_text());
        assert_eq!(
            submissions(&fixture),
            ["\"continue-fixture\""],
            "{markers:?}"
        );
    }
}

#[test]
fn long_title_border_does_not_hide_the_composer_or_a_draft() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["long-title", "bare-composer"]);
    let output = guarded(&fixture);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let drafted = Fixture::new("claude");
    live(&drafted, &["long-title"]);
    fs::write(drafted.root().join("composer"), "user draft").unwrap();
    let output = guarded(&drafted);
    details(&output, "send-composer-not-empty");
    drafted.no_delivery();
    assert_eq!(composer(&drafted), "user draft");
}

#[test]
fn refusal_after_paste_removes_exactly_the_pasted_text() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "start-after-paste"]);
    let output = guarded(&fixture);
    let details = details(&output, "send-session-not-idle");
    assert_eq!(details["stage"], "submit");
    assert_eq!(details["pasted"], true);
    assert_eq!(details["submitted"], false);
    assert_eq!(details["cleared"], true, "{details}");
    assert_eq!(composer(&fixture), "");
    assert_eq!(backspaces(&fixture), "continue-fixture".len());
    assert!(submissions(&fixture).is_empty());
    let message = output.stdout_json()["error"]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("no longer holds"), "{message}");
}

#[test]
fn refusal_after_paste_never_clears_a_mixed_draft() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "inject-draft"]);
    let output = guarded(&fixture);
    let details = details(&output, "send-composer-mismatch");
    assert_eq!(details["pasted"], true);
    assert_eq!(details["cleared"], false);
    assert_eq!(backspaces(&fixture), 0);
    assert_eq!(composer(&fixture), "continue-fixture user addition");
    let message = output.stdout_json()["error"]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("may remain"), "{message}");
}

#[test]
fn refusal_after_paste_reports_text_that_could_not_be_removed() {
    let fixture = Fixture::new("claude");
    live(
        &fixture,
        &["bare-composer", "start-after-paste", "backspace-ignored"],
    );
    let output = guarded(&fixture);
    let details = details(&output, "send-session-not-idle");
    assert_eq!(details["pasted"], true);
    assert_eq!(details["cleared"], false);
    assert_eq!(composer(&fixture), "continue-fixture");
    let message = output.stdout_json()["error"]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("may remain"), "{message}");
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn refusal_after_paste_leaves_an_unobservable_composer_alone() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "fail-capture-after-paste"]);
    let output = guarded(&fixture);
    let details = details(&output, "send-composer-unobservable");
    assert_eq!(details["pasted"], true);
    assert_eq!(details["cleared"], false);
    assert_eq!(backspaces(&fixture), 0);
}
