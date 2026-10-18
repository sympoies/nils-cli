use nils_test_support::cmd::{self, CmdOptions};
use nils_test_support::fs as test_fs;
use nils_test_support::{GlobalStateLock, bin};
use pretty_assertions::{assert_eq, assert_ne};
use std::fs;

#[test]
fn isolated_profiles_strip_control_environment_preserve_shell_and_cleanup_auth() {
    let _lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let stub = temp.path().join("bin");
    let source = temp.path().join("source");
    fs::create_dir_all(&stub).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("auth.json"), "credential-canary").unwrap();
    for entry in [
        "AGENTS.md",
        "config.toml",
        "hooks.json",
        "skills",
        "plugins",
        "agents",
        "memories",
        "goals",
    ] {
        if ["skills", "plugins", "agents", "memories", "goals"].contains(&entry) {
            fs::create_dir_all(source.join(entry)).unwrap();
        } else {
            fs::write(source.join(entry), "runtime-canary").unwrap();
        }
    }
    test_fs::write_executable(
        &stub.join("codex"),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = exec ] && [ "${2:-}" = --help ]; then
  printf '%s\n' '--ignore-user-config --ignore-rules --ephemeral --skip-git-repo-check --disable'; exit 0
fi
if [ "${1:-}" = features ]; then
  printf '%s\n' 'hooks plugins remote_plugin apps memories goals multi_agent workspace_dependencies shell_tool unified_exec'; exit 0
fi
# No control-plane family survives into the model child.
test "${CODEX_AUTH_FILE-unset}" = unset
test "${CODEX_SECRET_DIR-unset}" = unset
test "${CODEX_SECRET_CACHE_DIR-unset}" = unset
test "${CODEX_AUTH_REMOTE_TEST-unset}" = unset
test "${AGENT_DOCS_TEST-unset}" = unset
test "${AGENT_SESSION_TEST-unset}" = unset
test "${AGENT_HOOK_TEST-unset}" = unset
test "${AGENT_EVIDENCE_TEST-unset}" = unset
test "$HOME" = "$EXPECTED_HOME"
test "$GIT_CONFIG_COUNT" = 1
test "$GIT_CONFIG_KEY_0" = user.name
test "$GIT_CONFIG_VALUE_0" = 'Fixture Identity'
test "$SSH_AUTH_SOCK" = signing-agent-sentinel
test -L "$CODEX_HOME/auth.json"
test "$(find "$CODEX_HOME" -mindepth 1 -maxdepth 1 | wc -l | tr -d ' ')" = 1
printf '%s\n' "$CODEX_HOME" > "$CAPTURE_HOME"
for arg in "$@"; do printf '%s\n' "$arg"; done
# Exercise the replacement warning without changing the source credential.
unlink "$CODEX_HOME/auth.json"
printf '%s\n' child-replacement > "$CODEX_HOME/auth.json"
exit 23
"#,
    );
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let capture = temp.path().join("capture");
    let cwd = temp.path().join("project/child");
    fs::create_dir_all(&cwd).unwrap();
    let skills = [
        home.join(".agents/skills/home-agent/SKILL.md"),
        home.join(".codex/skills/home-codex/SKILL.md"),
        source.join("skills/source-home/SKILL.md"),
        cwd.join(".agents/skills/project-agent/SKILL.md"),
        cwd.join(".codex/skills/project-codex/SKILL.md"),
        temp.path()
            .join("project/.agents/skills/ancestor-agent/SKILL.md"),
        temp.path()
            .join("project/.codex/skills/ancestor-codex/SKILL.md"),
    ];
    for skill in &skills {
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(skill, "skill-canary").unwrap();
    }
    let options = CmdOptions::default()
        .with_cwd(&cwd)
        .with_path_prepend(&stub)
        .with_env("CODEX_HOME", &source.to_string_lossy())
        .with_env(
            "CODEX_AUTH_FILE",
            &source.join("auth.json").to_string_lossy(),
        )
        .with_env("HOME", &home.to_string_lossy())
        .with_env("EXPECTED_HOME", &home.to_string_lossy())
        .with_env("CAPTURE_HOME", &capture.to_string_lossy())
        .with_env("CODEX_SECRET_DIR", "control-sentinel")
        .with_env("CODEX_SECRET_CACHE_DIR", "control-sentinel")
        .with_env("CODEX_AUTH_REMOTE_TEST", "control-sentinel")
        .with_env("AGENT_DOCS_TEST", "control-sentinel")
        .with_env("AGENT_SESSION_TEST", "control-sentinel")
        .with_env("AGENT_HOOK_TEST", "control-sentinel")
        .with_env("AGENT_EVIDENCE_TEST", "control-sentinel")
        .with_env("GIT_CONFIG_COUNT", "1")
        .with_env("GIT_CONFIG_KEY_0", "user.name")
        .with_env("GIT_CONFIG_VALUE_0", "Fixture Identity")
        .with_env("SSH_AUTH_SOCK", "signing-agent-sentinel")
        .with_env_remove("CODEX_CLI_AGENT_RUNTIME");
    let mut homes = Vec::new();
    for (profile, sandbox) in [
        ("prompt", "workspace-write"),
        ("advice", "read-only"),
        ("knowledge", "read-only"),
    ] {
        let output = cmd::run_with(
            &bin::resolve("codex-cli"),
            &["agent", profile, "--ephemeral", "fixture request"],
            &options,
        );
        assert_eq!(output.code, 23, "{}", output.stderr_text());
        let args = output.stdout_text();
        assert!(args.contains(&format!("--sandbox\n{sandbox}\n")));
        for feature in [
            "hooks",
            "plugins",
            "remote_plugin",
            "apps",
            "memories",
            "goals",
            "multi_agent",
            "workspace_dependencies",
        ] {
            assert!(args.contains(&format!("--disable\n{feature}\n")));
        }
        assert!(args.contains("skills.include_instructions=false"));
        assert!(args.contains("skills.bundled.enabled=false"));
        assert!(args.contains("cloud.skills.enabled=false"));
        assert!(args.contains("skills.config="));
        for skill in &skills {
            let path = skill.canonicalize().unwrap();
            assert!(args.contains(&format!(
                "path={},enabled=false",
                toml::Value::String(path.to_str().unwrap().to_string())
            )));
        }
        assert!(args.contains("enabled=false"));
        assert!(
            output
                .stderr_text()
                .contains("isolated-auth-write-not-propagated")
        );
        assert!(!output.stderr_text().contains("credential-canary"));
        assert!(!args.contains("credential-canary"));
        let child = fs::read_to_string(&capture).unwrap();
        assert!(!std::path::Path::new(child.trim()).exists());
        assert!(!homes.contains(&child));
        homes.push(child);
        assert_eq!(
            fs::read_to_string(source.join("auth.json")).unwrap(),
            "credential-canary"
        );
    }
}

#[test]
fn unsupported_isolation_fails_before_model_and_never_uses_inherited_runtime() {
    let _lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let stub = temp.path().join("bin");
    fs::create_dir_all(&stub).unwrap();
    test_fs::write_executable(
        &stub.join("codex"),
        r#"#!/bin/sh
if [ "${1:-}" = exec ] && [ "${2:-}" = --help ]; then
  printf '%s\n' '--ephemeral --skip-git-repo-check --disable'; exit 0
fi
if [ "${1:-}" = features ]; then printf '%s\n' 'hooks'; exit 0; fi
printf 'model launched\n' > "$MODEL_MARKER"
exit 0
"#,
    );
    let marker = temp.path().join("model");
    let output = cmd::run_with(
        &bin::resolve("codex-cli"),
        &["agent", "prompt", "fixture"],
        &CmdOptions::default()
            .with_cwd(temp.path())
            .with_path_prepend(&stub)
            .with_env("MODEL_MARKER", &marker.to_string_lossy())
            .with_env_remove("CODEX_CLI_AGENT_RUNTIME"),
    );
    assert_ne!(output.code, 0);
    assert!(
        output
            .stderr_text()
            .contains("isolated-runtime-unsupported")
    );
    assert!(!marker.exists());
}
