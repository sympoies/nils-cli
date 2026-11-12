//! Console delivery must submit or clear only its own collapsed paste.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn collapsed_fixture(owner_suffix: bool) -> DirectFixture {
    let fixture = DirectFixture::new("claude");
    let tmux = &fixture.state.tmux_bin;
    let label = if owner_suffix {
        "[Pasted text #1 +3 lines] owner addition"
    } else {
        "[Pasted text #1 +3 lines]"
    };
    let script = fs::read_to_string(tmux)
        .unwrap()
        .replace(
            "(root / 'composer').write_bytes((root / 'buffer').read_bytes())",
            &format!(
                "(root / 'composer').write_text('{label}')\n    (root / 'cursor').write_text('1')"
            ),
        )
        .replace(
            "    sys.exit(0)\nif args[0] == 'load-buffer':",
            r#"    if 'display-message' in args:
        cursor = int((root / 'cursor').read_text()) if (root / 'cursor').exists() else 0
        if (root / 'stale-home-cursor').exists():
            cursor = 1
            (root / 'stale-home-cursor').unlink()
        print(2 + (len(text) if cursor else 0), 1)
    sys.exit(0)
if args[0] == 'load-buffer':"#,
        )
        .replace(
            "if args[0] == 'send-keys':",
            r#"if args[0] == 'send-keys':
    if args[-1] == 'C-a':
        (root / 'cursor').write_text('0')
        (root / 'stale-home-cursor').touch()
        sys.exit(0)
    if args[-2:] == ['Right', 'Left']:
        (root / 'cursor').write_text('0'); sys.exit(0)
    if args[-1] == 'Right':
        (root / 'cursor').write_text('1'); sys.exit(0)
    if args[-1] == 'BSpace' and (root / 'composer').read_text() == '[Pasted text #1 +3 lines]':
        if (root / 'cursor').read_text() == '1':
            (root / 'composer').write_text('')
            (root / 'cursor').write_text('0')
        sys.exit(0)"#,
        );
    let script = script.replace("if args[0] == 'capture-pane':\n", r#"if args[0] == 'capture-pane':
    if '-e' in args and (root / 'buffer').exists() and (root / 'fail-first-post-paste').exists() and not (root / 'failed-once').exists():
        (root / 'failed-once').touch()
        sys.exit(1)
"#);
    fs::write(tmux, script).unwrap();
    fixture
}

#[tokio::test]
async fn console_collapsed_placeholder_recovers_swallowed_enter_without_replaying_text() {
    let fixture = collapsed_fixture(false);
    fs::write(fixture.tmp.path().join("swallow-first"), "").unwrap();
    let mut body = fixture.body("prompt/v2");
    body["text"] = json!("first line\nsecond line\nthird line\nlast line");
    let (status, result) = fixture.submit("prompt/v2", body).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(
        fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
        ""
    );
    assert_eq!(
        fs::read_to_string(fixture.tmp.path().join("enter-count")).unwrap(),
        "2"
    );
    let calls = fs::read_to_string(fixture.tmp.path().join("transport-calls")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.starts_with("[\"paste-buffer\""))
            .count(),
        1
    );
}

#[tokio::test]
async fn console_collapsed_placeholder_is_cleared_when_all_enters_are_swallowed() {
    let fixture = collapsed_fixture(false);
    fs::write(fixture.tmp.path().join("stuck"), "").unwrap();
    let mut body = fixture.body("prompt/v2");
    body["text"] = json!("first line\nsecond line\nthird line\nlast line");
    let (status, result) = fixture.submit("prompt/v2", body).await;
    assert_ne!(status, StatusCode::OK, "{result}");
    assert_eq!(
        fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
        ""
    );
    assert!(!fixture.tmp.path().join("submitted").exists());
}

#[tokio::test]
async fn console_mixed_placeholder_preserves_owner_suffix_without_enter() {
    let fixture = collapsed_fixture(true);
    let mut body = fixture.body("prompt/v2");
    body["text"] = json!("first line\nsecond line\nthird line\nlast line");
    let (status, result) = fixture.submit("prompt/v2", body).await;
    assert_ne!(status, StatusCode::OK, "{result}");
    assert_eq!(
        fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
        "[Pasted text #1 +3 lines] owner addition"
    );
    assert!(!fixture.tmp.path().join("submitted").exists());
    assert!(!fixture.tmp.path().join("enter-count").exists());
}

#[tokio::test]
async fn console_placeholder_is_cleared_after_transient_first_observation() {
    let fixture = collapsed_fixture(false);
    fs::write(fixture.tmp.path().join("stuck"), "").unwrap();
    fs::write(fixture.tmp.path().join("fail-first-post-paste"), "").unwrap();
    let mut body = fixture.body("prompt/v2");
    body["text"] = json!("first line\nsecond line\nthird line\nlast line");
    let (status, result) = fixture.submit("prompt/v2", body).await;
    assert_ne!(status, StatusCode::OK, "{result}");
    assert_eq!(
        fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
        ""
    );
    assert!(!fixture.tmp.path().join("submitted").exists());
}

#[tokio::test]
async fn console_owned_placeholder_cleanup_tolerates_transport_latency() {
    for owner_suffix in [false, true] {
        let fixture = collapsed_fixture(owner_suffix);
        // Each response is well within the single-command timeout. The whole
        // ownership proof must also allow for these real subprocess round trips.
        fs::write(fixture.tmp.path().join("transport-delay-ms"), "80").unwrap();
        fs::write(fixture.tmp.path().join("stuck"), "").unwrap();
        let mut body = fixture.body("prompt/v2");
        body["text"] = json!("first line\nsecond line\nthird line\nlast line");
        let (status, result) = fixture.submit("prompt/v2", body).await;
        assert_ne!(status, StatusCode::OK, "{result}");
        assert_eq!(
            fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
            if owner_suffix {
                "[Pasted text #1 +3 lines] owner addition"
            } else {
                ""
            },
            "{result}"
        );
        assert!(!fixture.tmp.path().join("submitted").exists());
    }
}
