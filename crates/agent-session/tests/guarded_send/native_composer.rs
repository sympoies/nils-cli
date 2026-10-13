//! Guarded delivery against synthetic native blank-row and OSC-8 layout.
use super::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

#[test]
fn native_empty_layout_submits_exactly_once_and_rejects_the_same_key() {
    for provider in ["claude", "codex"] {
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
}

#[test]
fn native_layout_preserves_drafts_and_load_time_conflicts() {
    for provider in ["claude", "codex"] {
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
}
