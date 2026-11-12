use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn activity_setup_uses_selected_hook_and_rejects_path_and_old_override() {
    let temp = tempfile::tempdir().unwrap();
    let poison = temp.path().join("agent-hook");
    let marker = temp.path().join("executed");
    executable(&poison, "#!/bin/sh\nprintf poison > \"$MARKER\"\n");
    let selected = temp.path().join("selected");
    fs::create_dir(&selected).unwrap();
    executable(
        &selected.join("agent-hook"),
        "#!/bin/sh\nprintf selected > \"$MARKER\"\nexit 69\n",
    );
    let run = |root: &std::path::Path, old_override: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
        command
            .args([
                "activity",
                "setup",
                "--agent",
                "codex",
                "--dry-run",
                "--format",
                "json",
            ])
            .current_dir(temp.path())
            .env("PATH", temp.path())
            .env("MARKER", &marker)
            .env("AGENT_SESSION_BIN", "/nonexistent/agent-session")
            .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", root);
        if old_override {
            command.env("AGENT_HOOK_BIN", &poison);
        } else {
            command.env_remove("AGENT_HOOK_BIN");
        }
        command.output().unwrap()
    };
    for old_override in [false, true] {
        let output = run(&temp.path().join("missing"), old_override);
        assert!(!marker.exists(), "untrusted hook was executed");
        assert_eq!(output.status.code(), Some(69));
        assert!(String::from_utf8_lossy(&output.stdout).contains("agent-hook-setup-unavailable"));
    }
    let _ = run(&selected, true);
    assert_eq!(fs::read_to_string(marker).unwrap(), "selected");
}

#[test]
fn activity_setup_uses_the_hook_beside_the_configured_session_binary() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("executed");
    let installed = temp.path().join("installed");
    let other = temp.path().join("other");
    fs::create_dir_all(&installed).unwrap();
    fs::create_dir_all(&other).unwrap();
    // Another build's hook in a trusted root renders its own compile-time name.
    executable(
        &other.join("agent-hook"),
        "#!/bin/sh\nprintf other > \"$MARKER\"\nexit 69\n",
    );
    executable(
        &installed.join("agent-hook"),
        "#!/bin/sh\nprintf sibling > \"$MARKER\"\nexit 69\n",
    );
    let session = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &session).unwrap();
    Command::new(&session)
        .args([
            "activity",
            "setup",
            "--agent",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ])
        .current_dir(temp.path())
        .env("PATH", temp.path())
        .env("MARKER", &marker)
        .env_remove("AGENT_SESSION_BIN")
        .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", &other)
        .status()
        .unwrap();
    assert_eq!(fs::read_to_string(marker).unwrap(), "sibling");
}
