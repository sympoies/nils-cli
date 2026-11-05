//! Codex's blank composer rows hide typed spaces as well as layout cells.
use super::*;
use pretty_assertions::assert_eq;

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
