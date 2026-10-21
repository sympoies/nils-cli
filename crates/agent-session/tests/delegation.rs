use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn mint_requires_the_callers_runtime_credential() {
    let state = tempfile::TempDir::new().unwrap();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .arg("--state-dir")
        .arg(state.path())
        .args([
            "delegation",
            "mint",
            "--session",
            "caller",
            "--operation",
            "start-via-console",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "coordination-unauthorized");
    assert_eq!(
        envelope["schema_version"],
        "cli.agent-session.delegation-mint.v1"
    );
    assert!(!state.path().join("coordination").exists());
}
