//! A known paste may wrap visually without changing its logical input.
use super::{Fixture, composer, details, guarded_text, live, submissions};
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
            details(&output, "send-composer-unobservable")["submitted"],
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
fn wrapped_text_without_empty_composer_admission_never_submits() {
    let (fixture, text) = wrapped_fixture(44, "");
    let output = fixture.send_text(Some(&text), &["--require-idle"]);
    details(&output, "send-composer-unobservable");
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
        details(&output, "send-composer-unobservable");
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
