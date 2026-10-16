//! Guarded delivery against a synthetic Codex OSC-8 footer.
use super::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

#[test]
fn codex_hyperlink_footer_submits_once_and_rejects_the_same_key() {
    let provider = "codex";
    let fixture = Fixture::new(provider);
    fs::write(fixture.root().join("native-layout"), "").unwrap();
    let guards = [
        "--expect-runtime",
        "runtime-one",
        "--expect-provider-session",
        "provider-one",
        "--require-idle",
        "--require-empty-composer",
        "--idempotency-key",
        "native-empty",
    ];
    let output = fixture.send_text(Some("continue-fixture  "), &guards);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
    let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
    assert_eq!(submissions.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<String>(submissions.trim()).unwrap(),
        "continue-fixture  "
    );
    fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions")).unwrap(),
        submissions
    );
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|c| c[0] == "paste-buffer")
            .count(),
        1
    );
}

#[test]
fn codex_hyperlink_footer_preserves_drafts_and_load_time_conflicts() {
    let provider = "codex";
    for draft in ["user draft", " ", "first\n  second"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("native-layout"), "").unwrap();
        fs::write(fixture.root().join("composer"), draft).unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        fixture.refused(
            &output,
            if draft.contains('\n') {
                "send-composer-unobservable"
            } else {
                "send-composer-not-empty"
            },
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            draft
        );
    }
    for (marker, reason) in [
        ("draft-on-load", "send-composer-not-empty"),
        ("start-on-load", "send-session-not-idle"),
    ] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("native-layout"), "").unwrap();
        fs::write(fixture.root().join(marker), "").unwrap();
        fixture.refused(
            &fixture.send(&["--require-idle", "--require-empty-composer"]),
            reason,
        );
        fixture.no_delivery();
    }
}

#[test]
fn synthetic_dim_suggestion_sends_once_and_identical_human_drafts_block() {
    for provider in ["claude", "codex"] {
        let suggestion = "Continue reviewing the pending changes";
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("suggestion"), suggestion).unwrap();
        let guards = [
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "dim-suggestion",
        ];
        let output = fixture.send_text(Some("Proceed with the synthetic check"), &guards);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["outcome"],
            "submitted"
        );
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");

        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), suggestion).unwrap();
        fixture.refused(&fixture.send(&guards), "send-composer-not-empty");
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            suggestion
        );
    }
}
