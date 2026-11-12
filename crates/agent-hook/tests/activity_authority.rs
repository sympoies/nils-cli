mod support;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use nils_test_support::cmd::{CmdOptions, run_with};
use pretty_assertions::assert_eq;
use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"
[[rules]]
id = "coord.activity.prompt"
products = ["claude"]
events = ["UserPromptSubmit"]
priority = 100
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

fn helper(path: &Path, mode: u32, code: u8) {
    fs::write(path, format!("#!/bin/sh\nexit {code}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn activity_uses_trusted_sibling_and_rejects_untrusted_or_missing_siblings() {
    let fixture = Fixture::new(POLICY);
    let release = fixture.root.join("release");
    let poison = fixture.root.join("poison");
    fs::create_dir_all(&release).unwrap();
    fs::create_dir_all(&poison).unwrap();
    let hook = release.join("agent-hook");
    fs::copy(env!("CARGO_BIN_EXE_agent-hook"), &hook).unwrap();
    helper(&poison.join(support::SESSION_BIN), 0o700, 65);
    let sibling = release.join(support::SESSION_BIN);
    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_env_remove("AGENT_RUNTIME_TRUSTED_CLI_ROOT")
        .with_env("HOME", fixture.home.to_str().unwrap())
        .with_env("XDG_CONFIG_HOME", fixture.config_home.to_str().unwrap())
        .with_env("XDG_STATE_HOME", fixture.state_home.to_str().unwrap())
        .with_env(
            "AGENT_SESSION_STATE_DIR",
            fixture.session_state.to_str().unwrap(),
        )
        .with_env("AGENT_SESSION_ID", "managed-session")
        .with_env("AGENT_SESSION_RUNTIME_ID", "managed-runtime")
        .with_env("AGENT_SESSION_BIN", "/missing/agent-session")
        .with_env("PATH", poison.to_str().unwrap())
        .with_stdin_str(r#"{"hook_event_name":"UserPromptSubmit","prompt":"status"}"#);
    let run = || {
        run_with(
            &hook,
            &["dispatch", "--product", "claude", "--format", "json"],
            &options,
        )
    };
    helper(&sibling, 0o700, 0);
    assert_eq!(
        run().stdout_json()["data"]["reasons"][0]["code"],
        "activity-recorded"
    );
    for mode in [0o720, 0o600] {
        helper(&sibling, mode, 0);
        assert_eq!(
            run().stdout_json()["data"]["reasons"][0]["code"],
            "activity-helper-unresolvable"
        );
    }
    fs::remove_file(sibling).unwrap();
    assert_eq!(
        run().stdout_json()["data"]["reasons"][0]["code"],
        "activity-helper-unresolvable"
    );
}

#[test]
fn doctor_and_dispatch_use_ordered_roots_without_path_fallback() {
    let fixture = Fixture::new(POLICY);
    let release = fixture.root.join("release");
    let first = fixture.root.join("first");
    let second = fixture.root.join("second");
    let poison = fixture.root.join("poison");
    for dir in [&release, &first, &second, &poison] {
        fs::create_dir_all(dir).unwrap();
    }
    let hook = release.join("agent-hook");
    fs::copy(env!("CARGO_BIN_EXE_agent-hook"), &hook).unwrap();
    let target = fixture.root.join("selected-helper");
    helper(&target, 0o700, 0);
    symlink(&target, first.join(support::SESSION_BIN)).unwrap();
    helper(&second.join(support::SESSION_BIN), 0o700, 65);
    helper(&poison.join(support::SESSION_BIN), 0o700, 65);
    let preview = fixture.run(
        &[
            "setup",
            "--product",
            "claude",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    let digest = preview.stdout_json()["data"]["plan_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        fixture
            .run(
                &[
                    "setup",
                    "--product",
                    "claude",
                    "--apply",
                    "--expected-plan-digest",
                    &digest,
                    "--format",
                    "json"
                ],
                None
            )
            .code,
        0
    );
    let roots = std::env::join_paths([&first, &second]).unwrap();
    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        // `fixture.run` drops these provider config roots for setup; doctor
        // must read the same fixture config, not a host's.
        .with_env_remove_many(&["CLAUDE_CONFIG_DIR", "CODEX_HOME"])
        .with_env("HOME", fixture.home.to_str().unwrap())
        .with_env("XDG_CONFIG_HOME", fixture.config_home.to_str().unwrap())
        .with_env("XDG_STATE_HOME", fixture.state_home.to_str().unwrap())
        .with_env(
            "AGENT_SESSION_STATE_DIR",
            fixture.session_state.to_str().unwrap(),
        )
        .with_env("AGENT_SESSION_ID", "managed-session")
        .with_env("AGENT_SESSION_RUNTIME_ID", "managed-runtime")
        .with_env_os("AGENT_RUNTIME_TRUSTED_CLI_ROOT".as_ref(), &roots)
        .with_env("PATH", poison.to_str().unwrap());
    let dispatch = options
        .clone()
        .with_stdin_str(r#"{"hook_event_name":"UserPromptSubmit","prompt":"status"}"#);
    assert_eq!(
        run_with(
            &hook,
            &["dispatch", "--product", "claude", "--format", "json"],
            &dispatch
        )
        .stdout_json()["data"]["reasons"][0]["code"],
        "activity-recorded"
    );
    let doctor = || {
        run_with(
            &hook,
            &["doctor", "--product", "claude", "--format", "json"],
            &options,
        )
    };
    assert_eq!(doctor().code, 0);
    helper(&release.join(support::SESSION_BIN), 0o700, 0);
    fs::remove_file(first.join(support::SESSION_BIN)).unwrap();
    fs::remove_file(second.join(support::SESSION_BIN)).unwrap();
    assert_eq!(
        doctor().stdout_json()["error"]["code"],
        "activity-helper-unresolvable"
    );
}
