//! Exact synthetic rows and cursor positions from the resident composer's captures.
use super::{Fixture, composer, details_any, guarded_text, submissions};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;

fn captured(case_name: &str) -> (Fixture, String, Value) {
    let captures: Value =
        serde_json::from_str(include_str!("wrapped-provider-captures.json")).unwrap();
    let case = captures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case"] == case_name)
        .unwrap();
    let fixture = Fixture::new(case["provider"].as_str().unwrap());
    let text = case["text"].as_str().unwrap().to_string();
    assert_eq!(text.chars().count(), 190);
    let rows = case["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    // Only composer rows were captured. Supply generic surrounding chrome;
    // preserve each row's ANSI styling and trailing spaces verbatim.
    let pane = if case["provider"] == "claude" {
        format!("────────────────────\n{rows}\n────────────────────\n  status")
    } else {
        format!("header\n{rows}\n\n  status")
    };
    let position = |name: &str| json!([case[name][0], case[name][1].as_u64().unwrap() + 1]);
    let snapshot = json!({
        "input": text, "pane": pane, "start": position("home"),
        "before_end": position("before_end"), "prefix_end": position("end")
    });
    assert_eq!(position("after_paste"), snapshot["prefix_end"]);
    save(&fixture, &snapshot);
    (fixture, text, snapshot)
}

fn save(fixture: &Fixture, snapshot: &Value) {
    fs::write(
        fixture.root().join("captured-layout"),
        serde_json::to_vec(snapshot).unwrap(),
    )
    .unwrap();
}

fn submits_once(case: &str) {
    let (fixture, text, _) = captured(case);
    let output = guarded_text(&fixture, &text);
    assert_eq!(output.code, 0, "{case}: {}", output.stdout_text());
    let submitted: Vec<String> = submissions(&fixture)
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(submitted, [text]);
    assert_eq!(composer(&fixture), "");
    let calls = fixture.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    let keys: Vec<_> = calls
        .iter()
        .filter(|call| call[0] == "send-keys")
        .map(|call| call.last().unwrap().as_str())
        .collect();
    assert_eq!(keys, ["C-a", "Right", "Right", "Enter"]);
}

#[test]
fn codex_captured_ascii_word_wrap_with_hidden_trailing_space_submits_once() {
    submits_once("codex ascii");
}

#[test]
fn codex_captured_cjk_wrap_submits_once() {
    submits_once("codex cjk");
}

#[test]
fn claude_captured_cjk_wrap_preserves_asymmetric_padding() {
    submits_once("claude cjk");
}

#[test]
fn codex_captured_wrap_requires_empty_admission_and_rejects_an_appended_draft() {
    for draft in [false, true] {
        let (fixture, text, mut snapshot) = captured("codex cjk");
        let output = if draft {
            snapshot["input"] = format!("{text} owner draft").into();
            snapshot["pane"] = snapshot["pane"]
                .as_str()
                .unwrap()
                .replace("\n\n  status", " owner draft\n\n  status")
                .into();
            save(&fixture, &snapshot);
            fs::write(fixture.root().join("inject-draft"), "").unwrap();
            fs::write(fixture.root().join("draft-text"), " owner draft").unwrap();
            guarded_text(&fixture, &text)
        } else {
            fixture.send_text(Some(&text), &["--require-idle"])
        };
        let receipt = details_any(
            &output,
            &[
                "send-composer-mismatch",
                "send-composer-unobservable",
                "send-guard-observation-timeout",
            ],
        );
        assert_eq!(receipt["submitted"], false);
        assert!(submissions(&fixture).is_empty());
        assert!(
            fixture
                .calls()
                .iter()
                .all(|call| call.last().is_none_or(|key| key != "Enter"))
        );
        if draft {
            assert_eq!(composer(&fixture), format!("{text} owner draft"));
        }
    }
}

#[test]
fn codex_captured_wrap_requires_matching_layout_and_logical_cursor_path() {
    for variant in [
        "indent",
        "dim",
        "dialog",
        "blank",
        "home",
        "missing-space",
        "extra-space",
    ] {
        let (fixture, text, mut snapshot) = captured("codex ascii");
        let pane = snapshot["pane"].as_str().unwrap();
        snapshot["pane"] = match variant {
            "indent" => pane.replacen("\n  beta", "\n beta", 1),
            "dim" => pane.replace("alpha", "\x1b[2malpha\x1b[22m"),
            "dialog" => format!("{pane}\n  enter to confirm"),
            "blank" => pane.replacen("\n  beta", "\n\n  beta", 1),
            _ => pane.to_string(),
        }
        .into();
        if variant == "home" {
            snapshot["start"] = json!([2, 2]);
        }
        if matches!(variant, "missing-space" | "extra-space") {
            // Identical visible rows can hide a different logical separator.
            let boundary = snapshot["pane"].as_str().unwrap().lines().nth(1).unwrap();
            let boundary = boundary.split("\x1b[0m ").nth(1).unwrap().len();
            assert_eq!(&text[boundary..boundary + 1], " ");
            let actual = if variant == "missing-space" {
                format!("{}{}", &text[..boundary], &text[boundary + 1..])
            } else {
                format!("{} {}", &text[..boundary], &text[boundary..])
            };
            fs::write(fixture.root().join("paste-replacement"), &actual).unwrap();
            snapshot["input"] = actual.into();
        }
        save(&fixture, &snapshot);
        let output = guarded_text(&fixture, &text);
        let receipt = details_any(
            &output,
            &[
                "send-composer-unobservable",
                "send-guard-observation-timeout",
            ],
        );
        assert_eq!(receipt["submitted"], false, "{variant}");
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
