//! Claude shows a dim suggestion in an idle composer; one that starts like a
//! version must not read as a selection list that replaced the composer.
use super::*;
use pretty_assertions::assert_eq;

const RULE: &str = "────────────────────────────";

/// A fixture whose idle pane shows `row` after the prompt glyph while the composer is empty.
fn idle_with(row: &str) -> DirectFixture {
    let fixture = DirectFixture::new("claude");
    let tmux = fixture.tmp.path().join("guard-tmux");
    let script = fs::read_to_string(&tmux).unwrap().replacen(
        "if args[0] == 'capture-pane':\n",
        "if args[0] == 'capture-pane':\n    if (root / 'ghost').exists() and (root / 'composer').read_text() == '': print((root / 'ghost').read_text()); sys.exit(0)\n",
        1,
    );
    fs::write(&tmux, script).unwrap();
    fs::write(
        fixture.tmp.path().join("ghost"),
        format!("{RULE}\n❯\u{a0}{row}\n{RULE}\n  status"),
    )
    .unwrap();
    fixture
}

#[tokio::test]
async fn direct_claude_version_like_suggestion_in_an_idle_composer_delivers() {
    let suggestion = "\u{1b}[2m1.30.5\u{1b}[0m \u{1b}[2mnotes\u{1b}[0m";
    for route in ["prompt", "prompt/v2"] {
        let fixture = idle_with(suggestion);
        let (status, sent) = fixture.submit(route, fixture.body(route)).await;
        assert_eq!(status, StatusCode::OK, "{route}: {sent}");
        assert_eq!(sent["data"]["submitted"], true, "{route}");
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1, "{route}");
    }
}

#[tokio::test]
async fn direct_claude_numbered_choice_row_stays_refused() {
    // A selection list puts whitespace after the number, dim or not.
    for row in ["1. PRIVATE-MARKER", "\u{1b}[2m1. PRIVATE-MARKER\u{1b}[0m"] {
        let fixture = idle_with(row);
        let (status, refusal) = fixture.submit("prompt", fixture.body("prompt")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{row:?}: {refusal}");
        assert_eq!(refusal["error"]["code"], "composer-unobservable");
        assert_eq!(
            refusal["error"]["details"]["observation"],
            "composer-not-located"
        );
        assert!(!refusal.to_string().contains("PRIVATE-MARKER"));
        assert!(!fixture.tmp.path().join("buffer").exists());
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
    }
}
