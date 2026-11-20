use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;

pub const SYSTEM_PATH: &str = "/usr/bin:/bin";

/// Give a serve child only fixture configuration, including the tmux binary
/// used by startup maintenance before any sessions have been launched.
pub fn command(
    root: &Path,
    binary: impl AsRef<OsStr>,
    state: &Path,
    tmux: &Path,
    configure: impl FnOnce(&mut Command),
) -> Command {
    let home = root.join("home");
    let config = root.join("config");
    let data = root.join("data");
    let temp = root.join("tmp");
    let socket = root.join("tmux.sock");
    let tmux_env = format!("{},0,0", socket.display());

    let mut command = Command::new(binary);
    command
        .arg("serve")
        .env_clear()
        .env("PATH", SYSTEM_PATH)
        .env("HOME", &home)
        .env("AGENT_HOME", root.join("agent-home"))
        .env("TMPDIR", &temp)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_CONFIG_DIRS", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", &data)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_RUNTIME_DIR", root)
        .env("AGENT_SESSION_STATE_DIR", state)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("AGENT_SESSION_CODEX_BIN", root.join("unused-provider"))
        .env("AGENT_SESSION_CLAUDE_BIN", root.join("unused-provider"))
        .env("AGENT_SESSION_TMUX_BIN", tmux)
        .env("TMUX_TMPDIR", root)
        .env("TMUX", &tmux_env);
    configure(&mut command);

    let root = root.canonicalize().expect("resolve serve fixture root");
    let env = |key: &str| {
        command
            .get_envs()
            .find_map(|(name, value)| (name == key).then_some(value).flatten())
            .unwrap_or_else(|| panic!("missing serve fixture environment variable {key}"))
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
        "TMUX_TMPDIR",
    ] {
        assert_inside(&root, key, Path::new(env(key)));
    }
    assert_inside(
        &root,
        "session temp root",
        &Path::new(env("AGENT_HOME")).join("tmp"),
    );
    assert_inside(&root, "TMUX socket", &socket);
    assert_eq!(
        env("TMUX"),
        OsStr::new(&tmux_env),
        "use the private tmux socket"
    );
    assert_eq!(env("AGENT_SESSION_TMUX_BIN"), tmux.as_os_str());
    assert!(tmux.is_file(), "fixture must supply its tmux executable");
    fs::create_dir_all(Path::new(env("TMPDIR"))).expect("create serve fixture temp directory");
    command
}

fn assert_inside(root: &Path, name: &str, path: &Path) {
    assert!(
        path.is_absolute() && !path.components().any(|part| part == Component::ParentDir),
        "{name} must be an absolute fixture path: {path:?}"
    );
    // Some roots are created lazily by serve. Resolve their existing parent
    // so a fixture symlink cannot send a future write outside the fixture.
    let resolved = path
        .ancestors()
        .find(|ancestor| ancestor.symlink_metadata().is_ok())
        .and_then(|ancestor| ancestor.canonicalize().ok());
    assert!(
        resolved.is_some_and(|resolved| resolved.starts_with(root)),
        "{name} must resolve inside the serve fixture: {path:?}"
    );
}
