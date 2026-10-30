//! Retained synthetic wrapped ASCII regression and logical-space proof controls.
use super::super::{Fixture, composer, details, details_any, guarded_text, live, submissions};
use super::{captured, save};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;

fn ascii() -> (Fixture, String, Value) {
    let capture: Value = serde_json::from_str(include_str!("claude-ascii-wrap.json")).unwrap();
    let text = capture["text"].as_str().unwrap().to_owned();
    assert_eq!(text.chars().count(), 315);
    let rows = capture["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer"]);
    // Only the after-paste cursor is retained native evidence. Model C-a
    // and N-1 against the logical input; do not claim native key observations.
    let snapshot = json!({
        "input": text,
        "pane": format!("────────────────────\n{rows}\n────────────────────\n  status"),
        "start": [2, 1], "before_end": [25, 5],
        "prefix_end": [capture["after_paste"][0], capture["after_paste"][1].as_u64().unwrap() + 1]
    });
    save(&fixture, &snapshot);
    (fixture, text, snapshot)
}

fn assert_sent(fixture: &Fixture, text: &str) {
    let output = guarded_text(fixture, text);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
    assert_eq!(
        output.stdout_json()["data"]["submission"]["outcome"],
        "submitted"
    );
    let submitted: Vec<String> = submissions(fixture)
        .iter()
        .map(|row| serde_json::from_str(row).unwrap())
        .collect();
    assert_eq!(submitted, [text]);
    assert_eq!(composer(fixture), "");
    let calls = fixture.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call[0] == "send-keys" && call.last().unwrap() == "Enter")
            .count(),
        1
    );
    let retry = guarded_text(fixture, text);
    details(&retry, "send-idempotency-key-used");
    assert_eq!(submissions(fixture).len(), 1);
    assert!(
        fixture.calls()[calls.len()..]
            .iter()
            .all(|call| !matches!(call[0].as_str(), "paste-buffer" | "send-keys"))
    );
}

#[test]
fn claude_retained_ascii_wrapped_trailing_space_submits_once() {
    let (fixture, text, _) = ascii();
    assert_sent(&fixture, &text);
}

#[test]
fn claude_wide_wrapped_trailing_space_uses_character_steps_and_cell_endpoints() {
    let (fixture, text, mut snapshot) = captured("claude cjk");
    let text = format!("{text} ");
    snapshot["input"] = text.clone().into();
    // 38 wide characters occupy 76 cells; one known space reaches x79.
    snapshot["before_end"] = json!([78, 5]);
    snapshot["prefix_end"] = json!([79, 5]);
    save(&fixture, &snapshot);
    assert_sent(&fixture, &text);
}

#[test]
fn claude_wrapped_invisible_appended_space_never_submits() {
    // Both without and with a known trailing separator: N-1/N alone can
    // prove a prefix but cannot prove that the logical input ends there.
    for trailing in [false, true] {
        let (fixture, original, mut snapshot) = ascii();
        let text = if trailing {
            original
        } else {
            original.trim_end().to_owned()
        };
        let actual = format!("{text} ");
        let end = if trailing { 26 } else { 25 };
        snapshot["input"] = actual.clone().into();
        snapshot["before_end"] = json!([end, 5]);
        snapshot["prefix_end"] = json!([end + 1, 5]);
        snapshot["cursor_positions"] = json!({});
        snapshot["cursor_positions"][(text.len() - 1).to_string()] = json!([end - 1, 5]);
        snapshot["cursor_positions"][text.len().to_string()] = json!([end, 5]);
        save(&fixture, &snapshot);
        fs::write(fixture.root().join("inject-draft"), "").unwrap();
        fs::write(fixture.root().join("draft-text"), " ").unwrap();
        let output = guarded_text(&fixture, &text);
        let receipt = details_any(
            &output,
            &[
                "send-composer-unobservable",
                "send-guard-observation-timeout",
            ],
        );
        assert_eq!(receipt["submitted"], false);
        assert!(submissions(&fixture).is_empty());
        // Cleanup may remove the proven paste prefix, but must preserve
        // the extra owner space; it must never erase it as renderer padding.
        assert!(composer(&fixture).ends_with(' '));
        assert!(
            fixture
                .calls()
                .iter()
                .all(|call| call.last().is_none_or(|key| key != "Enter"))
        );
    }
}

#[test]
fn claude_retained_wrapped_trailing_space_is_removed_only_after_owned_cursor_proof() {
    let (fixture, text, _) = ascii();
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    let output = guarded_text(&fixture, &text);
    let receipt = details(&output, "send-session-not-idle");
    assert_eq!(receipt["submitted"], false);
    assert_eq!(receipt["cleared"], true, "{receipt}");
    assert_eq!(receipt["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );
}
