//! Model a synchronous provider ingress before its input enters readable history.
use super::{Fixture, GUARDS, Server};
use pretty_assertions::assert_eq;
use std::{fs, os::fd::AsRawFd, path::Path};

pub(super) fn record_available(session_dir: &Path) -> bool {
    let state = session_dir.parent().unwrap().parent().unwrap();
    let lock = fs::File::open(state.join("session-locks/guarded.lock")).unwrap();
    // SAFETY: the fixture owns this descriptor; no blocking lock or alternate inode.
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

pub(super) fn ingest_after_lock(session_dir: &Path, replace: bool) -> bool {
    if !record_available(session_dir) {
        return false;
    }
    let gate = fs::File::open(session_dir.join(".codex-app-server-manual-input-gate")).unwrap();
    // SAFETY: this independent fixture descriptor only probes the shared gate.
    assert_ne!(
        unsafe { libc::flock(gate.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "account mutation remains fenced throughout confirmation"
    );
    // A provider hook may update the record before making the input visible.
    let path = session_dir.join("session.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["title"] = serde_json::json!("provider ingress committed");
    if replace {
        record["runtime"]["launch_id"] = serde_json::json!("replacement-runtime");
        record["provider_resume"]["session_id"] = serde_json::json!("replacement-provider");
    }
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    true
}

#[test]
fn native_confirmation_allows_provider_ingress_after_ack_without_overwriting_it() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "hook-lock");
    let output = fixture.send(GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["native"]["confirmation"],
        "input_observed"
    );
    assert_eq!(
        fixture.document("session.json")["title"],
        "provider ingress committed"
    );
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert_eq!(server.mutations().len(), 1);
}

#[test]
fn native_identity_replaced_during_confirmation_never_returns_a_success_receipt() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "hook-lock-replaced");
    let value = fixture.refused(&fixture.send(GUARDS), "send-native-unconfirmed");
    assert_eq!(value["error"]["details"]["acknowledged"], true);
    assert_eq!(value["error"]["details"]["submitted"], true);
    assert_eq!(value["error"]["details"]["pasted"], false);
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
    assert!(
        fs::read_dir(fixture.root().join("state/sessions/guarded"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("send-key-"))
    );
}
