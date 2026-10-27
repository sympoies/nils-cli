//! A known paste may wrap visually without changing its logical input.
use super::{Fixture, composer, details, details_any, guarded_text, live, submissions};
use pretty_assertions::assert_eq;

fn wrapped_fixture(width: usize, draft: &str) -> (Fixture, String) {
    let text = "A generic fixture message verifies that narrow panes accept the complete owned paste while preserving any user draft.".to_string();
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer"]);
    let input = format!("{text}{draft}");
    let mut rows = vec![String::new()];
    for word in input.split(' ') {
        let row = rows.last_mut().unwrap();
        if !row.is_empty() && row.len() + 1 + word.len() > width - 2 {
            rows.push(word.to_string());
        } else {
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(word);
        }
    }
    let border = "─".repeat(width);
    let mut pane = format!("{border}\n");
    for (index, row) in rows.iter().enumerate() {
        pane.push_str(if index == 0 { "❯ " } else { "  " });
        pane.push_str(&format!("{row:<cells$}\n", cells = width - 2));
    }
    pane.push_str(&format!("{border}\n  status"));
    std::fs::write(
        fixture.root().join("captured-layout"),
        serde_json::to_vec(&serde_json::json!({
            "input": input, "pane": pane, "start": [2, 1],
            "before_end": [1 + rows.last().unwrap().len(), rows.len()],
            "prefix_end": [2 + rows.last().unwrap().len(), rows.len()]
        }))
        .unwrap(),
    )
    .unwrap();
    if !draft.is_empty() {
        std::fs::write(fixture.root().join("draft-text"), draft).unwrap();
        std::fs::write(fixture.root().join("inject-draft"), "").unwrap();
    }
    (fixture, text)
}

#[test]
fn owned_word_wrapped_paste_submits_once_at_narrow_widths() {
    for width in [32, 44, 70, 80] {
        let (fixture, text) = wrapped_fixture(width, "");
        let output = guarded_text(&fixture, &text);
        assert_eq!(output.code, 0, "width {width}: {}", output.stdout_text());
        assert_eq!(
            submissions(&fixture),
            [serde_json::to_string(&text).unwrap()]
        );
        assert_eq!(composer(&fixture), "");
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "send-keys" && call.last().unwrap() == "Enter")
                .count(),
            1
        );
    }
}

#[test]
fn wrapped_draft_collision_never_submits_at_narrow_widths() {
    for width in [32, 44, 70, 80] {
        let (fixture, text) = wrapped_fixture(width, " user draft");
        let output = guarded_text(&fixture, &text);
        assert_eq!(output.code, 1, "width {width}: {}", output.stdout_text());
        assert_eq!(
            details_any(
                &output,
                &[
                    "send-composer-mismatch",
                    "send-composer-unobservable",
                    "send-guard-observation-timeout",
                ],
            )["submitted"],
            false
        );
        assert!(submissions(&fixture).is_empty());
        assert!(composer(&fixture).ends_with(" user draft"));
        assert!(
            fixture
                .calls()
                .iter()
                .all(|call| call.last().is_none_or(|key| key != "Enter"))
        );
    }
}

#[test]
fn wrapped_paste_with_unproven_logical_start_never_submits() {
    let (fixture, text) = wrapped_fixture(44, "");
    let path = fixture.root().join("captured-layout");
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // C-a lands on the continuation row instead of the logical start.
    snapshot["start"] = serde_json::json!([2, 2]);
    std::fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let output = guarded_text(&fixture, &text);
    let details = details(&output, "send-composer-unobservable");
    assert_eq!(details["submitted"], false);
    // Cleanup also needs a proven start; retain the paste if C-a is unreliable.
    assert_eq!(details["cleared"], false);
    assert_eq!(composer(&fixture), text);
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );
}

#[test]
fn slow_wrapped_composer_proof_stops_at_retry_deadline() {
    let (fixture, text) = wrapped_fixture(44, "");
    std::fs::write(fixture.root().join("fail-post-paste-capture-once"), "3").unwrap();
    std::fs::write(fixture.root().join("slow-right-observation-ms"), "300").unwrap();

    let output = guarded_text(&fixture, &text);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    let receipt = output.stdout_json();
    assert!(
        matches!(
            receipt["error"]["code"].as_str(),
            Some("send-composer-unobservable" | "send-guard-observation-timeout")
        ),
        "{receipt}"
    );
    assert_eq!(receipt["error"]["details"]["stage"], "submit");
    assert_eq!(receipt["error"]["details"]["pasted"], true);
    assert_eq!(receipt["error"]["details"]["submitted"], false);
    assert_eq!(receipt["error"]["details"]["cleared"], true, "{receipt}");
    assert_eq!(receipt["error"]["details"]["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );
    let retry = fixture.send(&["--idempotency-key", "live-text"]);
    details(&retry, "send-idempotency-key-used");
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn readable_draft_mismatch_never_retries_into_submit() {
    let (fixture, text) = wrapped_fixture(80, "");
    let (drafted, _) = wrapped_fixture(80, " user draft");
    let mismatch: serde_json::Value =
        serde_json::from_slice(&std::fs::read(drafted.root().join("captured-layout")).unwrap())
            .unwrap();
    let exact: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.root().join("captured-layout")).unwrap())
            .unwrap();
    std::fs::write(
        fixture.root().join("mismatch-layout"),
        serde_json::to_vec(&mismatch).unwrap(),
    )
    .unwrap();
    std::fs::write(
        fixture.root().join("exact-layout"),
        serde_json::to_vec(&exact).unwrap(),
    )
    .unwrap();
    std::fs::write(
        fixture
            .root()
            .join("composer-mismatch-disappears-before-retry"),
        "",
    )
    .unwrap();

    let output = guarded_text(&fixture, &text);
    let receipt = details(&output, "send-composer-mismatch");
    assert_eq!(receipt["submitted"], false);
    assert_eq!(receipt["cleared"], true);
    assert_eq!(composer(&fixture), "");
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );

    let retry = fixture.send(&["--idempotency-key", "live-text"]);
    details(&retry, "send-idempotency-key-used");
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn draft_appearing_during_cursor_proof_never_submits() {
    let (fixture, text) = wrapped_fixture(32, "");
    let (drafted, _) = wrapped_fixture(32, " user draft");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.root().join("captured-layout")).unwrap())
            .unwrap();
    let mut changed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(drafted.root().join("captured-layout")).unwrap())
            .unwrap();
    // Appending a draft keeps the paste's existing cursor positions valid.
    // The fresh input observation must catch the collision on the first Right.
    changed["cursor_positions"] = serde_json::json!({});
    changed["cursor_positions"][(text.len() - 1).to_string()] = original["before_end"].clone();
    changed["cursor_positions"][text.len().to_string()] = original["prefix_end"].clone();
    std::fs::write(
        fixture.root().join("right-layout"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    std::fs::write(fixture.root().join("draft-text"), " user draft").unwrap();
    live(&fixture, &["draft-on-right"]);
    let output = guarded_text(&fixture, &text);
    let details = details(&output, "send-composer-mismatch");
    assert_eq!(details["submitted"], false);
    assert_eq!(details["cleared"], true);
    assert_eq!(composer(&fixture), " user draft");
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );
}

#[test]
fn runtime_replacement_stops_cursor_proof_before_more_keys() {
    for completed_right_steps in [0, 1, 2] {
        let (fixture, text) = wrapped_fixture(44, "");
        if completed_right_steps == 0 {
            live(&fixture, &["replace-on-home"]);
        } else {
            std::fs::write(
                fixture.root().join("replace-after-right"),
                completed_right_steps.to_string(),
            )
            .unwrap();
        }
        let output = guarded_text(&fixture, &text);
        // A transient observation retry must stop with the more specific
        // identity refusal once the runtime replacement is visible.
        let details = details(&output, "send-identity-mismatch");
        assert_eq!(details["submitted"], false);
        assert_eq!(details["cleared"], false);
        assert_eq!(composer(&fixture), text);
        assert!(submissions(&fixture).is_empty());
        let keys: Vec<_> = fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "send-keys")
            .map(|call| call.last().unwrap().clone())
            .collect();
        assert_eq!(
            keys,
            ["C-a", "Right", "Right"][..1 + completed_right_steps],
            "replacement after {completed_right_steps} Right steps"
        );
    }
}

#[test]
fn wrapped_text_without_empty_composer_admission_never_submits() {
    let (fixture, text) = wrapped_fixture(44, "");
    let output = fixture.send_text(Some(&text), &["--require-idle"]);
    details_any(
        &output,
        &[
            "send-composer-unobservable",
            "send-guard-observation-timeout",
        ],
    );
    assert!(submissions(&fixture).is_empty());
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "Enter"))
    );
}

#[test]
fn uncertain_wrapped_rendering_never_submits() {
    for variant in ["dim", "dialog", "blank-row", "extra-space", "missing-space"] {
        let (fixture, text) = wrapped_fixture(44, if variant == "extra-space" { " " } else { "" });
        let path = fixture.root().join("captured-layout");
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if variant == "missing-space" {
            // Remove the omitted separator at the first visual wrap. The
            // visible rows still match; cursor movement must detect the loss.
            let first_row = snapshot["pane"].as_str().unwrap().lines().nth(1).unwrap();
            let boundary = first_row.strip_prefix("❯ ").unwrap().trim_end().len();
            assert_eq!(&text[boundary..boundary + 1], " ");
            let actual = format!("{}{}", &text[..boundary], &text[boundary + 1..]);
            std::fs::write(fixture.root().join("paste-replacement"), &actual).unwrap();
            snapshot["input"] = actual.into();
        }
        let pane = snapshot["pane"].as_str().unwrap();
        snapshot["pane"] = match variant {
            "dim" => pane
                .replace("❯ ", "❯ \x1b[2m")
                .replace("\n  status", "\x1b[22m\n  status"),
            "dialog" => format!("{pane}\n  enter to confirm"),
            "blank-row" => pane.replacen("\n  ", "\n  \n  ", 1),
            _ => pane.to_string(),
        }
        .into();
        std::fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let output = guarded_text(&fixture, &text);
        details_any(
            &output,
            &[
                "send-composer-unobservable",
                "send-guard-observation-timeout",
            ],
        );
        assert!(submissions(&fixture).is_empty(), "{variant}");
        assert!(
            fixture
                .calls()
                .iter()
                .all(|call| call.last().is_none_or(|key| key != "Enter")),
            "{variant}"
        );
    }
}

#[test]
fn owned_character_wrapped_paste_submits_once() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "wrap-80"]);
    let text = "x".repeat(190);
    let output = guarded_text(&fixture, &text);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        submissions(&fixture),
        [serde_json::to_string(&text).unwrap()]
    );
}
