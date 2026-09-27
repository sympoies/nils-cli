use std::process::Command;

use nils_test_support::bin;

#[test]
fn remote_mailbox_surface_is_discoverable() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["message", "send", "--help"])
        .output()
        .expect("agent-session help");
    assert!(String::from_utf8_lossy(&output.stdout).contains("--to-machine"));
}

/// The `main-agent` facade ships from `nils-main-agent`; its CLI help contract
/// is tested there. The operator docs stay here with the orchestration engine,
/// so this checks that they still publish the same readiness default.
#[test]
fn main_agent_docs_publish_bounded_readiness_default_and_launch_only_opt_out() {
    for (name, docs) in [
        ("README", include_str!("../README.md")),
        (
            "orchestration runbook",
            include_str!("../docs/runbooks/main-agent-orchestration.md"),
        ),
    ] {
        assert!(
            docs.contains("defaults to waiting up to 5 minutes"),
            "{name} must publish the same omitted readiness default as CLI help"
        );
        assert!(
            docs.contains("`--await-ready 0`"),
            "{name} must publish the explicit launch-only opt-out"
        );
    }
}
