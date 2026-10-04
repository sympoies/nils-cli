use std::process::Command;

#[test]
fn service_mailbox_http_acceptance() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/service-mailbox-acceptance.mjs");
    let mut command = Command::new("node");
    command
        .arg(fixture)
        .arg("--agent-session-bin")
        .arg(nils_test_support::bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .output()
        .expect("Node >=18 is required for HTTP fixtures");
    assert!(
        output.status.success(),
        "service fixture failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("fixture receipt");
    pretty_assertions::assert_eq!(receipt["status"], "passed");
}
