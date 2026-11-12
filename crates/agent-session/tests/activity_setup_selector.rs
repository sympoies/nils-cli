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

/// Runs `session` for a dry-run activity setup with a trusted root holding a
/// hook that records `root`; `pin` is the caller's `AGENT_SESSION_BIN`.
fn run_setup(session: &std::path::Path, temp: &std::path::Path, pin: Option<&str>) -> String {
    let marker = temp.join("executed");
    let _ = fs::remove_file(&marker);
    let roots = temp.join("roots");
    fs::create_dir_all(&roots).unwrap();
    executable(
        &roots.join("agent-hook"),
        "#!/bin/sh\nprintf root > \"$MARKER\"\nexit 69\n",
    );
    let mut command = Command::new(session);
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
        .current_dir(temp)
        .env("PATH", temp)
        .env("MARKER", &marker)
        .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", &roots);
    match pin {
        Some(value) => command.env("AGENT_SESSION_BIN", value),
        None => command.env_remove("AGENT_SESSION_BIN"),
    };
    // A just-copied executable can still be open for writing in a forked
    // sibling test; retry the spawn while the kernel reports it busy.
    for _ in 0..50 {
        match command.output() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            result => {
                result.unwrap();
                break;
            }
        }
    }
    fs::read_to_string(marker).unwrap_or_default()
}

#[test]
fn activity_setup_falls_back_to_trusted_roots_without_a_sibling_hook() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    let session = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &session).unwrap();
    assert_eq!(run_setup(&session, temp.path(), None), "root");
    assert_eq!(run_setup(&session, temp.path(), Some("")), "root");
}

#[test]
fn activity_setup_keeps_a_caller_pinned_session_binary_and_ignores_mismatched_names() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    executable(
        &installed.join("agent-hook"),
        "#!/bin/sh\nprintf sibling > \"$MARKER\"\nexit 69\n",
    );
    let named = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &named).unwrap();
    // A pin that selects nothing keeps the sibling out of play.
    assert_eq!(
        run_setup(&named, temp.path(), Some("/nonexistent/other-session")),
        "root"
    );
    // An executable not running under the configured name uses the trusted roots.
    let renamed = installed.join("session-under-test");
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &renamed).unwrap();
    assert_eq!(run_setup(&renamed, temp.path(), None), "root");
}

#[test]
fn activity_setup_ignores_a_group_or_world_writable_sibling_hook() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    let hook = installed.join("agent-hook");
    executable(&hook, "#!/bin/sh\nprintf sibling > \"$MARKER\"\nexit 69\n");
    let session = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &session).unwrap();
    for mode in [0o775, 0o757] {
        fs::set_permissions(&hook, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(run_setup(&session, temp.path(), None), "root", "{mode:o}");
    }
}

#[test]
fn activity_setup_treats_an_empty_pin_as_unset_next_to_a_sibling_hook() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    executable(
        &installed.join("agent-hook"),
        "#!/bin/sh\nprintf sibling > \"$MARKER\"\nexit 69\n",
    );
    let session = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &session).unwrap();
    assert_eq!(run_setup(&session, temp.path(), Some("")), "sibling");
}

#[test]
fn activity_setup_ignores_a_sibling_that_is_not_a_usable_hook() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    let session = installed.join(nils_common::session_binary::NAME);
    fs::copy(env!("CARGO_BIN_EXE_agent-session"), &session).unwrap();
    let hook = installed.join("agent-hook");
    // A directory named like the hook.
    fs::create_dir(&hook).unwrap();
    assert_eq!(run_setup(&session, temp.path(), None), "root");
    fs::remove_dir(&hook).unwrap();
    // A non-executable file.
    executable(&hook, "#!/bin/sh\nprintf sibling > \"$MARKER\"\n");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(run_setup(&session, temp.path(), None), "root");
    fs::remove_file(&hook).unwrap();
    // A link into another directory: the selector would refuse it.
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    executable(
        &elsewhere.join("agent-hook"),
        "#!/bin/sh\nprintf linked > \"$MARKER\"\n",
    );
    std::os::unix::fs::symlink(elsewhere.join("agent-hook"), &hook).unwrap();
    assert_eq!(run_setup(&session, temp.path(), None), "root");
}
