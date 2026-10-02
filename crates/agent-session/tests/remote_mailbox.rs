//! Real separate-root HTTP federation acceptance, shared with installed fleet checks.
use std::process::Command;

#[test]
fn two_root_http_federation_acceptance() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/remote-mailbox-acceptance.mjs");
    let agent_session_bin = nils_test_support::bin::resolve("agent-session");
    let mut command = Command::new("node");
    command
        .arg(&fixture)
        .arg("--agent-session-bin")
        .arg(&agent_session_bin);
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .output()
        .expect("Node >=18 is required for the installed-artifact HTTP fixture");
    assert!(
        output.status.success(),
        "HTTP fixture failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("fixture receipt");
    pretty_assertions::assert_eq!(receipt["status"], "passed");
}
