use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const CLAUDE_POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "fixture-claude-policy"
version = "2026.10.19.1"

[[rules]]
id = "fixture.claude-activity"
products = ["claude"]
events = ["UserPromptSubmit"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

struct ClaudeRegistrationFixture {
    _temp: tempfile::TempDir,
    config_dir: PathBuf,
    _env: Vec<EnvGuard>,
}

impl ClaudeRegistrationFixture {
    fn new(lock: &GlobalStateLock) -> Self {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = fs::canonicalize(temp.path()).expect("canonical fixture path");
        let home = root.join("home");
        let config_home = root.join("config");
        let state_home = root.join("state");
        let config_dir = root.join("claude-config");
        fs::create_dir_all(&home).expect("home directory");
        fs::create_dir_all(config_home.join("agent-hook")).expect("agent-hook config directory");
        fs::create_dir_all(&state_home).expect("state directory");
        fs::create_dir_all(&config_dir).expect("Claude config directory");
        let policy = root.join("policy.toml");
        fs::write(&policy, CLAUDE_POLICY).expect("policy bundle");
        let config = config_home.join("agent-hook/config.toml");
        fs::write(
            &config,
            format!(
                "schema_version = \"agent-hook.config.v1\"\n[policy]\npath = {}\ndigest = \"sha256:{}\"\n",
                serde_json::to_string(&policy).expect("policy path JSON"),
                Sha256::digest(CLAUDE_POLICY.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
        )
        .expect("agent-hook config");
        let dispatcher = nils_test_support::bin::required_sibling("agent-hook", "nils-agent-hook");
        let env = vec![
            EnvGuard::set(lock, "HOME", home.to_str().expect("home path")),
            EnvGuard::set(
                lock,
                "CLAUDE_CONFIG_DIR",
                config_dir.to_str().expect("Claude config path"),
            ),
            EnvGuard::set(
                lock,
                "XDG_CONFIG_HOME",
                config_home.to_str().expect("config home path"),
            ),
            EnvGuard::set(
                lock,
                "XDG_STATE_HOME",
                state_home.to_str().expect("state home path"),
            ),
            EnvGuard::set(
                lock,
                "AGENT_HOOK_BIN",
                dispatcher.to_str().expect("agent-hook binary path"),
            ),
        ];
        let code = agent_hook::run_with_args([
            "agent-hook",
            "setup",
            "--product",
            "claude",
            "--apply",
            "--format",
            "json",
        ]);
        assert_eq!(code, 0, "Claude dispatch setup must converge");
        Self {
            _temp: temp,
            config_dir,
            _env: env,
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }
}

#[test]
fn claude_config_path_uses_config_dir_override() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("claude-config");
    std::fs::create_dir_all(&home).expect("home directory");
    std::fs::create_dir_all(&config).expect("Claude config directory");
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().expect("home path"));
    let _config = EnvGuard::set(
        &lock,
        "CLAUDE_CONFIG_DIR",
        config.to_str().expect("Claude config path"),
    );

    assert_eq!(
        provider_config_path(AgentKind::Claude).expect("Claude config path"),
        config.join("settings.json")
    );
}

#[test]
fn codex_config_paths_use_codex_home_override() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let codex_home = temp.path().join("codex-config");
    std::fs::create_dir_all(&home).expect("home directory");
    std::fs::create_dir_all(&codex_home).expect("Codex config directory");
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().expect("home path"));
    let _codex_home = EnvGuard::set(
        &lock,
        "CODEX_HOME",
        codex_home.to_str().expect("Codex config path"),
    );

    assert_eq!(
        provider_config_path(AgentKind::Codex).expect("Codex hooks path"),
        codex_home.join("hooks.json")
    );
    assert_eq!(
        codex_notification_config_path().expect("Codex notification path"),
        codex_home.join("config.toml")
    );
}

#[test]
fn claude_dispatch_registration_is_configured() {
    let lock = GlobalStateLock::new();
    let fixture = ClaudeRegistrationFixture::new(&lock);

    let readiness = agent_hook_claude_readiness(&fixture.settings_path())
        .expect("valid agent-hook registration response");

    assert!(readiness.configured);
    assert!(readiness.missing_owned_events.is_empty());
}

#[test]
fn claude_missing_dispatch_event_is_reported_by_name() {
    let lock = GlobalStateLock::new();
    let fixture = ClaudeRegistrationFixture::new(&lock);
    let path = fixture.settings_path();
    let mut settings: Value =
        serde_json::from_slice(&fs::read(&path).expect("settings file")).expect("settings JSON");
    settings["hooks"]
        .as_object_mut()
        .expect("hooks object")
        .remove("UserPromptSubmit");
    fs::write(&path, serde_json::to_vec(&settings).expect("settings JSON"))
        .expect("remove one owned event");

    let readiness = agent_hook_claude_readiness(&path).expect("valid drifted setup response");

    assert!(!readiness.configured);
    assert_eq!(readiness.missing_owned_events, ["UserPromptSubmit"]);
}
