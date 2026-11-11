//! Codex's blank composer rows hide typed spaces as well as layout cells.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn composer(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.root().join("composer")).unwrap()
}

#[test]
fn codex_notice_preserves_invisible_separator_draft_and_consumes_key() {
    for (body, changed_tail) in [
        ("synthetic", " \nsynthetic"),
        (" \n\nsynthetic", "\n\n \nsynthetic"),
    ] {
        let fixture = Fixture::new("codex");
        fs::write(fixture.root().join("notice-body"), body).unwrap();
        let changed = format!(
            "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n{changed_tail}"
        );
        fs::write(fixture.root().join("paste-replacement"), &changed).unwrap();
        let output = fixture.notice(&[]);
        let receipt = fixture.refused(&output, "send-composer-unobservable");
        assert_eq!(receipt["error"]["details"]["submitted"], false);
        assert_eq!(receipt["error"]["details"]["cleared"], false);
        assert_eq!(
            receipt["error"]["details"]["remaining_input"],
            "unobservable"
        );
        assert_eq!(composer(&fixture), changed);
        assert!(!fixture.root().join("submissions").exists());
        assert!(fixture.calls().iter().all(|call| {
            call[0] != "send-keys"
                || call
                    .last()
                    .is_none_or(|key| !matches!(key.as_str(), "Enter" | "BSpace" | "DC"))
        }));
        fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    }
}

#[test]
fn codex_notice_cleans_only_proven_paste_after_idle_conflict() {
    let fixture = Fixture::new("codex");
    fs::write(fixture.root().join("notice-body"), "synthetic").unwrap();
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    let output = fixture.notice(&[]);
    let receipt = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(receipt["error"]["details"]["submitted"], false);
    assert_eq!(receipt["error"]["details"]["cleared"], true);
    assert_eq!(receipt["error"]["details"]["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn codex_notice_preserves_readable_owner_suffix() {
    let fixture = Fixture::new("codex");
    fs::write(fixture.root().join("notice-body"), "synthetic").unwrap();
    fs::write(fixture.root().join("inject-draft"), "").unwrap();
    let output = fixture.notice(&[]);
    let receipt = fixture.refused(&output, "send-composer-mismatch");
    assert_eq!(
        receipt["error"]["details"]["remaining_input"],
        "different_text"
    );
    assert_eq!(receipt["error"]["details"]["cleared"], false);
    assert!(composer(&fixture).ends_with("synthetic user addition"));
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn codex_notice_preserves_owned_blank_spaces_and_final_newline() {
    let fixture = Fixture::new("codex");
    let body = "  \n \nsynthetic \n";
    fs::write(fixture.root().join("notice-body"), body).unwrap();
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    let output = fixture.notice(&[]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let submitted: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        submitted,
        [format!(
            "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\n{body}"
        )]
    );
}

fn collapsed_notice_fixture(body: &str) -> (Fixture, String) {
    let fixture = Fixture::new("codex");
    fs::write(fixture.root().join("notice-body"), body).unwrap();
    fs::write(fixture.root().join("codex-collapse-paste"), "").unwrap();
    let envelope = format!(
        "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\n{body}"
    )
    .replace("\r\n", "\n")
    .replace('\r', "\n");
    (fixture, envelope)
}

#[test]
fn codex_collapsed_notice_submits_once_with_normalized_unicode_character_count() {
    // Codex 0.162.1 apply_paste normalizes CRLF/CR then counts Unicode
    // scalars, not bytes, graphemes or display cells. The first case matches
    // the resident's single 1153-character placeholder and two-line body.
    for body in [
        format!("{}\n{}", "x".repeat(539), "y".repeat(538)),
        format!("{}\r\nsecond line", "界e\u{301}🙂".repeat(300)),
    ] {
        let (fixture, envelope) = collapsed_notice_fixture(&body);
        fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
        let output = fixture.notice(&[]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert_eq!(submissions, [envelope]);
        assert_eq!(composer(&fixture), "");
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
        fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    }
}

#[test]
fn codex_collapsed_notice_cleans_one_owned_token_after_idle_conflict() {
    let (fixture, _) = collapsed_notice_fixture(&"x".repeat(1100));
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    let output = fixture.notice(&[]);
    let receipt = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(receipt["error"]["details"]["cleared"], true);
    assert_eq!(receipt["error"]["details"]["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(!fixture.root().join("submissions").exists());
    let deletions: Vec<_> = fixture
        .calls()
        .into_iter()
        .filter(|call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "BSpace"))
        .collect();
    assert_eq!(deletions.len(), 1);
    assert_eq!(deletions[0][deletions[0].len() - 2], "1");
}

#[test]
fn codex_collapsed_notice_preserves_changed_or_nonatomic_drafts() {
    let body = "x".repeat(1100);
    let marker = "[Pasted Content 1175 chars]";
    for (draft, atomic) in [
        (format!("{marker} owner text"), true),
        ("[Pasted Content 1176 chars]".to_owned(), true),
        (format!("{marker}{marker}"), true),
        (marker.to_owned(), false),
    ] {
        let (fixture, _) = collapsed_notice_fixture(&body);
        if !atomic {
            fs::remove_file(fixture.root().join("codex-collapse-paste")).unwrap();
        }
        fs::write(fixture.root().join("paste-replacement"), &draft).unwrap();
        let output = fixture.notice(&[]);
        assert_eq!(output.code, 1, "{}", output.stdout_text());
        assert_eq!(output.stdout_json()["error"]["details"]["submitted"], false);
        assert_eq!(output.stdout_json()["error"]["details"]["cleared"], false);
        assert_eq!(composer(&fixture), draft);
        assert!(!fixture.root().join("submissions").exists());
        assert!(fixture.calls().iter().all(|call| {
            call[0] != "send-keys"
                || call
                    .last()
                    .is_none_or(|key| !matches!(key.as_str(), "Enter" | "BSpace" | "DC"))
        }));
        fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    }
}
// Codex 0.162.1 renders soft word wraps as separate capture rows, including
// with tmux -J. Generic synthetic text, a 64-column pane and origin row nine
// reproduce the native composer layout without retaining surrounding content.
fn wrapped_notice_fixture(hard_break: bool) -> (Fixture, String) {
    let fixture = Fixture::new("codex");
    let header = "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->";
    let body = "Synthetic first line.\nSynthetic second line.";
    let envelope = format!("{header}\n\n{body}");
    let input = if hard_break {
        envelope.replacen("scheduler id=", "scheduler\nid=", 1)
    } else {
        envelope.clone()
    };
    fs::write(fixture.root().join("notice-body"), body).unwrap();
    if hard_break {
        fs::write(fixture.root().join("paste-replacement"), &input).unwrap();
    }
    let pane = format!(
        "{}› <!-- agent-notice:v1 kind=schedule.tick source=scheduler\n  id=Example-1 -->\n \n  Synthetic first line.\n  Synthetic second line.\n \n  status\n",
        "\n".repeat(9)
    );
    let snapshot = json!({
        "input": input, "pane": pane, "start": [2, 9], "prefix_end": [24, 13],
        "cursor_positions": {
            "0": [2, 9], "56": [58, 9], "73": [18, 10],
            "74": [2, 11], "75": [2, 12], "96": [23, 12],
            "97": [2, 13], "119": [24, 13]
        }
    });
    fs::write(
        fixture.root().join("captured-layout"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    (fixture, input)
}

#[test]
fn codex_notice_wrapped_header_submits_once_at_native_origin() {
    let (fixture, envelope) = wrapped_notice_fixture(false);
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    let output = fixture.notice(&[]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
        .unwrap()
        .lines()
        .map(|row| serde_json::from_str(row).unwrap())
        .collect();
    assert_eq!(submissions, [envelope]);
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter"))
            .count(),
        1
    );
    fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
}

#[test]
fn codex_notice_wrapped_header_cleans_owned_paste_after_idle_conflict() {
    let (fixture, _) = wrapped_notice_fixture(false);
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    fs::write(fixture.root().join("slow-cleanup-capture"), "250").unwrap();
    let output = fixture.notice(&[]);
    let receipt = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(receipt["error"]["details"]["cleared"], true);
    assert_eq!(receipt["error"]["details"]["remaining_input"], "empty");
    assert_eq!(composer(&fixture), "");
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn codex_notice_wrapped_header_preserves_paste_when_cleanup_budget_expires() {
    let (fixture, input) = wrapped_notice_fixture(false);
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    fs::write(fixture.root().join("slow-cleanup-roundtrip"), "").unwrap();
    let started = std::time::Instant::now();
    let output = fixture.notice(&[]);
    assert!(started.elapsed() >= std::time::Duration::from_secs(15));
    let receipt = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(receipt["error"]["details"]["cleared"], false);
    assert_eq!(composer(&fixture), input);
    assert!(!fixture.root().join("submissions").exists());
    assert!(
        fixture.calls().iter().all(|call| {
            call[0] != "send-keys" || call.last().is_none_or(|key| key != "BSpace")
        })
    );
}

#[test]
fn codex_notice_wrapped_header_preserves_lookalike_hard_newline() {
    let (fixture, input) = wrapped_notice_fixture(true);
    let output = fixture.notice(&[]);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["error"]["details"]["submitted"], false);
    assert_eq!(output.stdout_json()["error"]["details"]["cleared"], false);
    assert_eq!(composer(&fixture), input);
    assert!(!fixture.root().join("submissions").exists());
    assert!(fixture.calls().iter().all(|call| {
        call[0] != "send-keys"
            || call
                .last()
                .is_none_or(|key| !matches!(key.as_str(), "Enter" | "BSpace" | "DC"))
    }));
}
