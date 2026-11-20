//! External Claude version boundaries use only synthetic provider binaries.
use super::*;
use pretty_assertions::assert_eq;

const DIALOG: &str = "Bash command\n  fixture-operation\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again\n  3. No\nEsc to cancel · Tab to amend\n";

fn pending(version: &str) -> Fixture {
    let f = Fixture::new("claude");
    set_version(&f, version);
    hook(&f, "UserPromptSubmit", None);
    hook(&f, "PreToolUse", Some("call-one"));
    hook(&f, "PermissionRequest", None);
    pane(&f, DIALOG);
    f
}

pub(super) fn set_version(f: &Fixture, version: &str) {
    let provider = fixture_version(f, version);
    let path = f.root().join("state/sessions/guarded/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["agent_bin"] = json!(provider);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

fn pane(f: &Fixture, screen: &str) {
    fs::write(
        f.root().join("captured-layout"),
        json!({"input":"", "pane":screen, "prefix_end":[2, 1]}).to_string(),
    )
    .unwrap();
}

fn approve(f: &Fixture) -> CmdOutput {
    f.send_text(
        None,
        &[
            "--expect-runtime",
            "runtime-one",
            "--expect-provider-session",
            "provider-one",
        ],
    )
}

fn admitted(f: &Fixture) {
    let out = approve(f);
    assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
    assert_eq!(
        f.calls()
            .iter()
            .filter(|c| c[0] == "send-keys" && c.last().unwrap() == "Enter")
            .count(),
        1
    );
    let receipt: Value = serde_json::from_slice(
        &fs::read(
            f.root()
                .join("state/sessions/guarded/permission-terminal-admission.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["reason"], "permission-provider-version-unsupported");
    assert_eq!(receipt["runtime_id"], "runtime-one");
    assert_eq!(receipt.as_object().unwrap().len(), 5);
    assert_eq!(
        receipt["schema_version"],
        "agent-session.permission-terminal-admission.v1"
    );
    assert_eq!(receipt["key"], "enter");
    assert!(receipt["checked_at"].is_string());
}

#[test]
fn claude_permission_below_fence_admits_guarded_dialog_key() {
    admitted(&pending("2.1.316"));
}

#[test]
fn claude_permission_above_fence_admits_guarded_dialog_key() {
    admitted(&pending("2.1.318"));
}

#[test]
fn claude_permission_titled_pane_admits_guarded_dialog_key() {
    let f = pending("2.1.325");
    pane(&f, &format!("{DIALOG}─── session title ─────────\n"));
    admitted(&f);
}

#[test]
fn claude_permission_in_fence_keeps_hook_only_even_with_visible_dialog() {
    let f = pending("2.1.317");
    f.refused(&approve(&f), "claude-permission-hook-required");
    f.no_delivery();
}

#[test]
fn claude_permission_unreadable_version_is_retryable_then_rechecks_binary() {
    let f = pending("unreadable");
    let body = f.refused(&approve(&f), "permission-provider-version-unavailable");
    assert_eq!(body["error"]["details"]["retryable"], true);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("attached terminal")
    );
    f.no_delivery();
    set_version(&f, "2.1.318");
    admitted(&f);
}

#[test]
fn claude_permission_outside_fence_refuses_composer_and_unreadable_dialog() {
    for screen in [
        "",
        "Do you want to proceed?\n❯ composer draft\n",
        "historical dialog\nDo you want to proceed?\n❯ 1. Yes\n  3. No\nEsc to cancel\n────────────────────────────\n❯ draft\n────────────────────────────",
        "Do you want to proceed?\n❯ 1. Yes\n─── unexpected rule ───\n3. No\nEsc to cancel\n─── session title ───",
        "Do you want to proceed?\n❯ 1. Yes\n3. No\nEsc to cancel\n❯ draft\n─── session title ───",
        "Do you want to proceed?\n❯ 1. Yes\n3. No\nEsc to cancel\n────────────────────────────",
    ] {
        let f = pending("2.1.318");
        pane(&f, screen);
        f.refused(&approve(&f), "permission-dialog-unobservable");
        f.no_delivery();
    }
    for flag in ["capture-fails", "capture-oversize"] {
        let f = pending("2.1.318");
        fs::write(f.root().join(flag), "").unwrap();
        f.refused(&approve(&f), "permission-dialog-unobservable");
        f.no_delivery();
    }
}

#[test]
fn claude_permission_outside_fence_refuses_text_even_with_allow_blocked() {
    let f = pending("2.1.318");
    f.refused(
        &f.send_text(Some("yes"), &["--allow-blocked"]),
        "permission-terminal-input-unsupported",
    );
    f.no_delivery();
    f.refused(
        &f.send_text(None, &["--key", "enter", "--expect-runtime", "runtime-one"]),
        "permission-terminal-input-unsupported",
    );
    f.no_delivery();
}
