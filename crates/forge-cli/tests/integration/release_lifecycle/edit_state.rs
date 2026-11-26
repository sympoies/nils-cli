use super::*;
use pretty_assertions::assert_eq;

fn payload(stub: &StubEnv) -> Value {
    serde_json::from_slice(&fs::read(stub.tempdir.path().join("payload.json")).unwrap()).unwrap()
}

#[test]
fn release_edit_drafts_and_publishes_the_same_release_idempotently() {
    let stub = fixture("evidence/check");
    for (flag, draft, count) in [("--draft", true, 1), ("--publish", false, 2)] {
        for _ in 0..2 {
            let d = data(&invoke(&stub, &["release", "edit", "evidence/check", flag]));
            assert_eq!(d["release_id"], 17);
            assert_eq!(d["tag"], "evidence/check");
            assert_eq!(d["draft"], draft);
            assert_eq!(d["notes"], "Original notes");
            assert_eq!(d["title"], "Original title");
            assert_eq!(writes(&stub).len(), count);
            assert_eq!(payload(&stub), json!({"draft": draft}));
        }
    }
}

#[test]
fn release_edit_publishes_an_untagged_draft_with_notes_and_title() {
    let stub = fixture("evidence-draft");
    let path = stub.tempdir.path().join("state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["release"]["draft"] = true.into();
    state["tag_exists"] = false.into();
    fs::write(&path, state.to_string()).unwrap();

    let args = [
        "release",
        "edit",
        "evidence-draft",
        "--publish",
        "--notes-file",
        "notes.md",
        "--title",
        TITLE,
    ];
    for _ in 0..2 {
        let d = data(&invoke(&stub, &args));
        assert_eq!(d["release_id"], 17);
        assert_eq!(d["tag"], "evidence-draft");
        assert_eq!(d["draft"], false);
        assert_eq!(d["target_commit"], "c".repeat(40));
        assert_eq!(d["notes"], NOTES);
        assert_eq!(d["title"], TITLE);
        assert_eq!(writes(&stub).len(), 1);
        assert_eq!(
            payload(&stub),
            json!({"draft": false, "body": NOTES, "name": TITLE})
        );
    }
}

#[test]
fn release_edit_rejects_unexpected_state_or_id_on_independent_readback() {
    for mode in ["edit-draft-readback-mismatch", "edit-id-readback-mismatch"] {
        let stub = fixture("evidence-check").env("RELEASE_MODE", mode);
        let result = invoke(&stub, &["release", "edit", "evidence-check", "--draft"]);
        let e = error(&result, "release_readback_mismatch");
        assert_eq!(e["details"]["retryable"], false);
        assert_eq!(writes(&stub).len(), 1);
    }
}

#[test]
fn release_edit_accepts_a_title_without_notes_or_a_state_change() {
    let stub = fixture("evidence-check");
    let d = data(&invoke(
        &stub,
        &["release", "edit", "evidence-check", "--title", TITLE],
    ));
    assert_eq!(d["title"], TITLE);
    assert_eq!(d["notes"], "Original notes");
    assert_eq!(d["draft"], false);
    assert_eq!(payload(&stub), json!({"name": TITLE}));
}

#[test]
fn release_edit_state_dry_run_and_invalid_requests_do_not_contact_the_provider() {
    let stub = fixture("evidence-check").env("FORGE_IDENTITY_PRINCIPAL", "missing-fixture");
    for (flag, draft) in [("--draft", true), ("--publish", false)] {
        let d = data(&invoke(
            &stub,
            &["release", "edit", "evidence-check", flag, "--dry-run"],
        ));
        assert_eq!(d["draft"], draft);
        assert!(d["follow_up"].to_string().contains("PATCH"));
    }
    let empty = invoke(&stub, &["release", "edit", "evidence-check"]);
    let e = error(&empty, "release_edit_empty");
    assert!(e["message"].as_str().unwrap().contains("--notes-file"));
    let conflict = invoke(
        &stub,
        &["release", "edit", "evidence-check", "--draft", "--publish"],
    );
    assert_ne!(conflict.code, 0);
    let envelope = parse_envelope(&conflict.stdout);
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(message.contains("--draft") && message.contains("--publish"));
    assert!(calls(&stub).is_empty());
}
