use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};

use super::fake_tmux;
use crate::serve_fixture;

pub(super) fn isolated_serve_command(
    fixture: &Path,
    daemon_root: &Path,
    state_dir: &Path,
    addr: SocketAddr,
) -> Command {
    let home = daemon_root.join("home");
    fs::create_dir_all(&home).expect("serve home");
    let (tmux, tmux_log) = fake_tmux(daemon_root);
    serve_fixture::command(
        fixture,
        nils_test_support::bin::resolve("agent-session"),
        state_dir,
        &tmux,
        |command| {
            command
                .args(["--bind", &addr.to_string()])
                .env("HOME", home)
                .env("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log)
                .env("AGENT_SESSION_FAKE_TMUX_LIST_WINDOWS", "")
                .stdin(Stdio::null())
                .stdout(Stdio::null());
        },
    )
}

pub(super) fn usage_serve_command(root: &Path, addr: SocketAddr) -> Command {
    let command = isolated_serve_command(root, root, &root.join("state"), addr);
    // Usage uses provider helpers and needs no terminal sessions. A daemon
    // probe can still be running when the fixture stops the daemon.
    super::write_executable(&root.join("tmux"), "#!/bin/sh\nexit 0\n");
    command
}

pub(super) fn assert_direct_serve_roots(fixture: &Path, command: &Command) {
    let fixture = fixture.canonicalize().expect("resolve exact TempDir");
    let assert_inside = |key: &str, value: &Path| {
        let resolved = value
            .ancestors()
            .find(|path| path.symlink_metadata().is_ok())
            .and_then(|path| path.canonicalize().ok());
        assert!(
            resolved.is_some_and(|path| path.starts_with(&fixture)),
            "{key} must resolve inside the exact TempDir"
        );
    };
    let env = |key: &str| {
        command
            .get_envs()
            .find_map(|(name, value)| (name == key).then_some(value).flatten())
            .unwrap_or_else(|| panic!("missing fixture environment variable {key}"))
    };
    for key in [
        "HOME",
        "AGENT_HOME",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "XDG_CONFIG_DIRS",
        "XDG_DATA_HOME",
        "XDG_DATA_DIRS",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "AGENT_SESSION_STATE_DIR",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "AGENT_SESSION_CODEX_BIN",
        "AGENT_SESSION_CLAUDE_BIN",
        "AGENT_SESSION_TMUX_BIN",
        "AGENT_SESSION_FAKE_TMUX_LOG",
        "TMUX_TMPDIR",
    ] {
        assert_inside(key, Path::new(env(key)));
    }
    let socket = env("TMUX")
        .to_str()
        .and_then(|value| value.strip_suffix(",0,0"))
        .expect("fixture TMUX socket");
    assert_inside("TMUX socket", Path::new(socket));
}
