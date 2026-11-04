//! Same-send ownership of a collapsed Claude paste.
use super::*;
use pretty_assertions::assert_eq;
const PLACEHOLDER: &str = "[Pasted text #1]";

#[test]
fn collapsed_paste_is_submitted_by_the_send_that_admitted_empty_input() {
    for guards in [
        &[][..],
        &["--require-empty-composer"][..],
        &["--expect-runtime", "runtime-one"][..],
    ] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("collapse-paste"), "").unwrap();
        let text = "x".repeat(801);
        let output = fixture.send_text(Some(&text), guards);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["outcome"],
            "submitted"
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[test]
fn collapsed_paste_is_cleared_when_submission_is_not_confirmed() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-paste"), "").unwrap();
    fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
    fs::write(fixture.root().join("late-home"), "").unwrap();
    let text = "x".repeat(801);
    let output = fixture.send_text(Some(&text), &["--require-empty-composer"]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["error"]["details"]["cleared"], true);
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        ""
    );
}

#[test]
fn preexisting_owner_paste_is_never_submitted_or_cleared() {
    for guards in [
        &[][..],
        &["--require-empty-composer"][..],
        &["--expect-runtime", "runtime-one"][..],
    ] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("composer"), PLACEHOLDER).unwrap();
        let text = "x".repeat(801);
        let output = fixture.send_text(Some(&text), guards);
        assert_eq!(output.code, 1, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["details"]["observation"],
            "paste-placeholder"
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            PLACEHOLDER
        );
    }
}

#[test]
fn plain_multiline_collapsed_paste_is_cleared_after_swallowed_enters() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-paste"), "").unwrap();
    fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
    fs::write(fixture.root().join("late-home"), "").unwrap();
    let output = fixture.send_text(Some("first line\nsecond line\nthird line\nlast line"), &[]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["error"]["details"]["cleared"], true);
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        ""
    );
}

#[test]
fn collapsed_paste_with_an_owner_suffix_is_preserved_without_enter() {
    for guards in [&[][..], &["--require-empty-composer"][..]] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("collapse-paste"), "").unwrap();
        fs::write(fixture.root().join("inject-draft"), "").unwrap();
        let text = "x".repeat(801);
        let output = fixture.send_text(Some(&text), guards);
        assert_eq!(output.code, 1, "{}", output.stdout_text());
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            format!("{PLACEHOLDER} user addition")
        );
        assert!(fixture.calls().iter().all(|call| {
            call.last()
                .is_none_or(|key| key != "Enter" && key != "BSpace")
        }));
    }
}

#[test]
fn placeholder_with_logical_owner_space_is_not_submitted_or_cleared() {
    for guards in [&[][..], &["--require-empty-composer"][..]] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("collapse-paste"), "").unwrap();
        fs::write(fixture.root().join("inject-draft"), "").unwrap();
        fs::write(fixture.root().join("draft-text"), " ").unwrap();
        let text = "x".repeat(801);
        let output = fixture.send_text(Some(&text), guards);
        assert_eq!(output.code, 1, "{}", output.stdout_text());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            format!("{PLACEHOLDER} ")
        );
        assert!(fixture.calls().iter().all(|call| {
            call.last()
                .is_none_or(|key| key != "Enter" && key != "BSpace")
        }));
    }
}

#[test]
fn placeholder_owner_suffix_after_recognition_is_preserved() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-paste"), "").unwrap();
    fs::write(fixture.root().join("draft-after-exact"), "").unwrap();
    fs::write(fixture.root().join("draft-text"), " owner addition").unwrap();
    let text = "x".repeat(801);
    let output = fixture.send_text(Some(&text), &[]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        format!("{PLACEHOLDER} owner addition")
    );
    assert!(fixture.calls().iter().all(|call| {
        call.last()
            .is_none_or(|key| key != "Enter" && key != "BSpace")
    }));
}

#[test]
fn placeholder_cleanup_preserves_owner_replacement_during_navigation() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-paste"), "").unwrap();
    fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
    fs::write(fixture.root().join("draft-on-clear-home"), "").unwrap();
    let text = "x".repeat(801);
    let output = fixture.send_text(Some(&text), &[]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        "replacement draft"
    );
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call.last().is_none_or(|key| key != "BSpace"))
    );
}

#[test]
fn plain_placeholder_transient_first_capture_preserves_owner_suffix() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-paste"), "").unwrap();
    fs::write(fixture.root().join("inject-draft"), "").unwrap();
    fs::write(fixture.root().join("draft-text"), " owner addition").unwrap();
    fs::write(fixture.root().join("fail-post-paste-capture-once"), "1").unwrap();
    let text = "x".repeat(801);
    let output = fixture.send_text(Some(&text), &[]);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        format!("{PLACEHOLDER} owner addition")
    );
    assert!(fixture.calls().iter().all(|call| {
        call.last()
            .is_none_or(|key| key != "Enter" && key != "BSpace")
    }));
}

#[test]
fn identity_guards_precede_owner_placeholder_admission() {
    for guard in ["--expect-runtime", "--expect-provider-session"] {
        for owner_input in ["owner draft", PLACEHOLDER] {
            let fixture = Fixture::new("claude");
            fs::write(fixture.root().join("composer"), owner_input).unwrap();
            let output = fixture.send(&[guard, "replaced-identity"]);
            fixture.refused(&output, "send-identity-mismatch");
            fixture.no_delivery();
            assert_eq!(
                fs::read_to_string(fixture.root().join("composer")).unwrap(),
                owner_input
            );
        }
    }
}

#[test]
fn placeholder_collapsing_after_exact_paste_is_cleared_on_refusal() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("collapse-after-exact"), "").unwrap();
    let output = fixture.send_text(
        Some(&"x".repeat(801)),
        &["--require-idle", "--require-empty-composer"],
    );
    let receipt = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(
        receipt["error"]["details"]["paste_observed_before_conflict"],
        true
    );
    assert_eq!(receipt["error"]["details"]["cleared"], true);
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        ""
    );
    assert!(!fixture.root().join("submissions").exists());
}
