//! Third-round regressions: word wrapping and an appended draft after paste.
use super::{Fixture, composer, details, guarded_text, live, submissions};
use pretty_assertions::assert_eq;

fn payload() -> String {
    let text = "A generic fixture message checks that a refused guarded send removes its own pasted text and preserves any user draft. ".repeat(3);
    text[..314].to_string()
}

#[test]
fn word_wrapped_paste_only_refusal_restores_empty_composer() {
    let text = payload();
    assert_eq!(text.len(), 314);
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "wrap-80", "word-wrap"]);
    let output = guarded_text(&fixture, &text);
    let receipt = details(&output, "send-composer-unobservable");
    assert_eq!(receipt["stage"], "submit");
    assert_eq!(receipt["pasted"], true);
    assert_eq!(receipt["submitted"], false);
    assert_eq!(receipt["cleared"], true, "{receipt}");
    assert_eq!(receipt["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn paste_then_draft_refusal_restores_only_the_user_draft() {
    // Injection is ordered after the exact-paste capture and before submit observation,
    // making the reported timing race deterministic without sleeps or live input.
    for text in ["short fixture".to_string(), payload()] {
        let fixture = Fixture::new("claude");
        live(
            &fixture,
            &["bare-composer", "wrap-80", "word-wrap", "inject-draft"],
        );
        let output = guarded_text(&fixture, &text);
        let code = if text.len() > 78 {
            "send-composer-unobservable"
        } else {
            "send-composer-mismatch"
        };
        let receipt = details(&output, code);
        assert_eq!(receipt["pasted"], true);
        assert_eq!(receipt["submitted"], false);
        assert_eq!(composer(&fixture), " user addition", "{receipt}");
        assert_eq!(receipt["cleared"], true);
        assert_eq!(receipt["remaining_input"], "different_text");
        assert_eq!(receipt["pasted_text_present"], false);
        assert!(submissions(&fixture).is_empty());
    }
}

#[test]
fn uncertain_navigation_changed_input_or_identity_never_deletes() {
    for marker in [
        "cursor-not-at-start",
        "wrong-prefix-end",
        "draft-on-home",
        "replace-on-home",
        "replace-after-paste",
    ] {
        let fixture = Fixture::new("claude");
        live(&fixture, &["bare-composer", "inject-draft", marker]);
        let output = guarded_text(&fixture, "short fixture");
        let receipt = details(&output, "send-composer-mismatch");
        assert_eq!(receipt["cleared"], false, "{marker}: {receipt}");
        assert!(fixture.calls().iter().all(|call| {
            call.last()
                .is_none_or(|key| !matches!(key.as_str(), "DC" | "BSpace"))
        }));
        assert!(submissions(&fixture).is_empty());
        assert_eq!(
            composer(&fixture),
            if matches!(marker, "draft-on-home" | "replace-after-paste") {
                "replacement draft"
            } else {
                "short fixture user addition"
            }
        );
    }
}

#[test]
fn a_draft_containing_the_same_text_is_preserved_as_user_input() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "inject-draft"]);
    std::fs::write(fixture.root().join("draft-text"), "short fixture").unwrap();
    let output = guarded_text(&fixture, "short fixture");
    let receipt = details(&output, "send-composer-mismatch");
    assert_eq!(receipt["cleared"], true);
    assert_eq!(receipt["pasted_text_present"], true); // content, not ownership
    assert_eq!(composer(&fixture), "short fixture");
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn captured_word_wrap_refusals_restore_empty_or_draft_only() {
    let captures: serde_json::Value =
        serde_json::from_str(include_str!("claude-wrapped-captures.json")).unwrap();
    for (index, case) in captures["cases"].as_array().unwrap().iter().enumerate() {
        let rows: Vec<&str> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row.as_str().unwrap())
            .collect();
        let (paste, draft) = if index == 0 {
            (rows.join(" "), "")
        } else {
            (format!("{} {}", rows[0], "x".repeat(5)), "xxxxxxxxxx")
        };
        if index == 0 {
            assert_eq!(paste.len(), 314);
        }
        let fixture = Fixture::new("claude");
        live(&fixture, &["bare-composer", "inject-draft"]);
        std::fs::write(fixture.root().join("draft-text"), draft).unwrap();
        let border = format!("\x1b[38;5;244m{}", "─".repeat(80));
        let mut pane = format!("{}{}\n\x1b[39m❯\u{a0}", "\n".repeat(12), border);
        for (row, content) in rows.iter().enumerate() {
            if row > 0 {
                pane.push_str("  ");
            }
            pane.push_str(content);
            pane.push_str(&" ".repeat(case["padding"][row].as_u64().unwrap() as usize));
            pane.push('\n');
        }
        pane.push_str(&format!("{border}\n\x1b[0m  \x1b[2mstatus\x1b[0m"));
        let snapshot = serde_json::json!({
            "input": format!("{paste}{draft}"),
            "pane": pane,
            "prefix_end": [if index == 0 { 15 } else { 7 }, 13 + rows.len() - 1]
        });
        std::fs::write(
            fixture.root().join("captured-layout"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let output = guarded_text(&fixture, &paste);
        let receipt = details(&output, "send-composer-unobservable");
        assert_eq!(receipt["cleared"], true, "case {index}: {receipt}");
        assert_eq!(composer(&fixture), draft);
        assert!(submissions(&fixture).is_empty());
    }
}

#[test]
fn delayed_navigation_and_transient_post_key_redraw_still_restore_the_draft() {
    for marker in ["late-home", "redraw-after-delete"] {
        let fixture = Fixture::new("claude");
        live(&fixture, &["bare-composer", "inject-draft", marker]);
        let output = guarded_text(&fixture, "short fixture");
        let receipt = details(&output, "send-composer-mismatch");
        assert_eq!(receipt["cleared"], true, "{marker}: {receipt}");
        assert_eq!(composer(&fixture), " user addition");
        assert!(submissions(&fixture).is_empty());
    }
}

#[test]
fn review_f1_draft_before_paste_with_same_prefix_is_never_deleted() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "draft-before-paste"]);
    let output = guarded_text(&fixture, "hello");
    let receipt = details(&output, "send-composer-mismatch");
    assert_eq!(composer(&fixture), "hello world hello");
    assert_eq!(receipt["cleared"], false);
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "BSpace"))
    );
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn review_f2_exact_single_row_with_trailing_spaces_uses_only_backspace() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "start-after-paste"]);
    let output = guarded_text(&fixture, "hello  ");
    let receipt = details(&output, "send-session-not-idle");
    assert_eq!(composer(&fixture), "");
    assert_eq!(receipt["cleared"], true);
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !matches!(call.last().map(String::as_str), Some("C-a" | "Right")))
    );
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn review_f3_wrapped_cleanup_bounds_tmux_roundtrips_without_timing_assertions() {
    let fixture = Fixture::new("claude");
    live(&fixture, &["bare-composer", "wrap-80", "word-wrap"]);
    let output = guarded_text(&fixture, &payload());
    let calls = fixture.calls();
    let paste = calls
        .iter()
        .position(|call| call[0] == "paste-buffer")
        .unwrap();
    let after_paste = calls.len() - paste - 1;
    assert!(
        after_paste <= 16,
        "{after_paste} post-paste tmux round trips: {calls:?}"
    );
    assert_eq!(
        details(&output, "send-composer-unobservable")["cleared"],
        true
    );
    assert_eq!(composer(&fixture), "");
}

#[test]
fn review_f1_immediate_unobserved_mixture_reports_unverified_ownership() {
    let fixture = Fixture::new("claude");
    std::fs::write(fixture.root().join("inject-draft"), "").unwrap();
    let output = guarded_text(&fixture, "hello");
    let receipt = details(&output, "send-composer-mismatch");
    assert_eq!(composer(&fixture), "hello user addition");
    assert_eq!(receipt["cleared"], false);
    assert_eq!(receipt["paste_observed_before_conflict"], false);
    let response = output.stdout_json();
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("paste ownership was not verified")
    );
    assert!(submissions(&fixture).is_empty());
}
