use super::{Peer, hash, send_notice};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

#[test]
fn idle_claude_with_prior_turns_delivers_natively_after_unknown_claims_fill_the_ledger() {
    let fixture = super::super::Fixture::new("claude");
    let peer = Peer::start(&fixture, "accepted");
    let root = fixture.root();
    let mut transcript = fs::OpenOptions::new()
        .append(true)
        .open(root.join("provider-config/projects/synthetic/provider-one.jsonl"))
        .unwrap();
    for _ in 0..512 {
        let row = json!({
            "type":"user", "sessionId":"provider-one", "cwd":root,
            "message":{"role":"user", "content":"Earlier synthetic turn"}
        });
        writeln!(transcript, "{row}").unwrap();
    }
    drop(transcript);
    let ready: Value = serde_json::from_slice(
        &fs::read(root.join("state/sessions/guarded/claude-notice/runtime-one/ready.json"))
            .unwrap(),
    )
    .unwrap();
    let parent = root.join("state/history/claude-notices");
    fs::create_dir_all(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = parent.join("ledger.jsonl");
    let mut bytes = Vec::new();
    let envelope = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nPrior notice\n";
    for index in 0..1024 {
        let op = format!("{index:08x}-0000-4000-8000-000000000000");
        let claim = json!({
            "schema_version":"agent-session.claude-notice-ledger.v1", "kind":"claim",
            "op":op, "binding":ready["binding"],
            "history_id":format!("h-{}", hash(b"claude\0\0provider-one")),
            "envelope_bytes":envelope.len(), "envelope_sha256":hash(envelope.as_bytes()),
            "original_budget_ms":15000,
            "baseline":{"device":0,"inode":0,"eof":0,"continuity_sha256":hash(b"")}
        });
        let claim_line = serde_json::to_string(&claim).unwrap();
        let terminal = json!({
            "schema_version":"agent-session.claude-notice-ledger.v1", "kind":"terminal",
            "op":op, "claim_sha256":hash(claim_line.as_bytes()), "state":"unknown", "reason":"guard"
        });
        writeln!(bytes, "{claim_line}\n{terminal}").unwrap();
    }
    fs::write(&ledger, &bytes).unwrap();
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();

    let output = send_notice(&fixture, "Capacity fixture notice\n");
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(
        output.stdout_json()["data"]["native"]["transport"],
        "claude_mod"
    );
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        0
    );
    assert_eq!(peer.text.lock().unwrap().len(), 1);
    assert_eq!(fs::read(&ledger).unwrap(), bytes);
    fixture.no_delivery();
}
