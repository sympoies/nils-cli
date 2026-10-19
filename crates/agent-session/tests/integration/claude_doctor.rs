use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::Value;
use sha2::{Digest, Sha256};

use nils_test_support::cmd::CmdOutput;

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

[[rules]]
id = "fixture.claude-pre-tool-write"
products = ["claude"]
events = ["PreToolUse"]
matcher = "Write"
priority = 20
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }

[[rules]]
id = "fixture.claude-pre-tool-edit"
products = ["claude"]
events = ["PreToolUse"]
matcher = "Edit"
priority = 30
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

struct ClaudeDoctorFixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    config_home: PathBuf,
    state_home: PathBuf,
    claude_config: PathBuf,
    codex_home: PathBuf,
    settings_path: PathBuf,
    agent_hook: PathBuf,
}

impl ClaudeDoctorFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = fs::canonicalize(temp.path()).expect("canonical fixture root");
        let home = root.join("home");
        let config_home = root.join("config");
        let state_home = root.join("state");
        let claude_config = home.join("claude-config");
        let codex_home = home.join("codex-config");
        let agent_hook_config_dir = config_home.join("agent-hook");
        fs::create_dir_all(&home).expect("home directory");
        fs::create_dir_all(&config_home).expect("config home");
        fs::create_dir_all(&state_home).expect("state home");
        fs::create_dir_all(&claude_config).expect("Claude config");
        fs::create_dir_all(&codex_home).expect("Codex config");
        fs::create_dir_all(&agent_hook_config_dir).expect("agent-hook config");

        let policy = root.join("claude-policy.toml");
        fs::write(&policy, CLAUDE_POLICY).expect("Claude activity policy");
        let policy_digest = Sha256::digest(CLAUDE_POLICY.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(
            agent_hook_config_dir.join("config.toml"),
            format!(
                "schema_version = \"agent-hook.config.v1\"\n[policy]\npath = {}\ndigest = \"sha256:{}\"\n",
                serde_json::to_string(policy.to_str().expect("policy path"))
                    .expect("quoted policy path"),
                policy_digest
            ),
        )
        .expect("agent-hook config");

        let agent_hook = nils_test_support::bin::required_sibling("agent-hook", "nils-agent-hook");
        let setup = Command::new(&agent_hook)
            .args([
                "setup",
                "--product",
                "claude",
                "--apply",
                "--format",
                "json",
            ])
            .env("HOME", &home)
            .env("CLAUDE_CONFIG_DIR", &claude_config)
            .env("CODEX_HOME", &codex_home)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_STATE_HOME", &state_home)
            .env_remove("AGENT_HOOK_BIN")
            .output()
            .expect("run agent-hook setup");
        assert!(
            setup.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&setup.stdout),
            String::from_utf8_lossy(&setup.stderr)
        );

        Self {
            _temp: temp,
            root,
            home,
            config_home,
            state_home,
            claude_config: claude_config.clone(),
            codex_home,
            settings_path: claude_config.join("settings.json"),
            agent_hook,
        }
    }

    fn doctor(&self, agent_hook: &Path) -> CmdOutput {
        let home = self.home.to_string_lossy().into_owned();
        let claude_config = self.claude_config.to_string_lossy().into_owned();
        let codex_home = self.codex_home.to_string_lossy().into_owned();
        let config_home = self.config_home.to_string_lossy().into_owned();
        let state_home = self.state_home.to_string_lossy().into_owned();
        let agent_hook = agent_hook.to_string_lossy().into_owned();
        super::cli::run(
            &self.root,
            &[
                "activity", "doctor", "--agent", "claude", "--format", "json",
            ],
            &[
                ("HOME", home.as_str()),
                ("CLAUDE_CONFIG_DIR", claude_config.as_str()),
                ("CODEX_HOME", codex_home.as_str()),
                ("XDG_CONFIG_HOME", config_home.as_str()),
                ("XDG_STATE_HOME", state_home.as_str()),
                ("AGENT_HOOK_BIN", agent_hook.as_str()),
            ],
        )
    }
}

fn claude_provider(output: &CmdOutput) -> Value {
    let json = output.stdout_json();
    super::cli::data(&json)["providers"][0].clone()
}

fn assert_configuration_error(output: &CmdOutput, expected: &str) {
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let provider = claude_provider(output);
    assert_eq!(provider["configured"], false);
    assert_eq!(provider["configuration_error"], expected);
    assert!(
        provider["guidance"]
            .as_str()
            .expect("guidance")
            .contains(expected)
    );
}

#[test]
fn claude_activity_doctor_reports_registration_and_configuration_states() {
    let fixture = ClaudeDoctorFixture::new();
    let original_settings = fs::read(&fixture.settings_path).expect("installed settings");

    let healthy = fixture.doctor(&fixture.agent_hook);
    assert_eq!(healthy.code, 0, "stderr={}", healthy.stderr_text());
    let healthy_provider = claude_provider(&healthy);
    assert_eq!(healthy_provider["configured"], true);
    assert_eq!(
        healthy_provider["config_path"],
        fixture
            .settings_path
            .to_str()
            .expect("Claude settings path")
    );
    assert!(healthy_provider.get("missing_owned_events").is_none());
    assert!(healthy_provider.get("configuration_error").is_none());
    assert!(
        !healthy_provider["guidance"]
            .as_str()
            .expect("guidance")
            .contains("Missing agent-hook owned events")
    );

    let mut settings: Value =
        serde_json::from_slice(&original_settings).expect("Claude settings JSON");
    settings["hooks"]
        .as_object_mut()
        .expect("hooks object")
        .remove("PreToolUse")
        .expect("PreToolUse groups");
    fs::write(
        &fixture.settings_path,
        serde_json::to_vec_pretty(&settings).expect("settings JSON"),
    )
    .expect("remove owned event groups");
    let drifted = fixture.doctor(&fixture.agent_hook);
    assert_eq!(drifted.code, 0, "stderr={}", drifted.stderr_text());
    let drifted_provider = claude_provider(&drifted);
    assert_eq!(drifted_provider["configured"], false);
    assert_eq!(
        drifted_provider["missing_owned_events"],
        serde_json::json!(["PreToolUse"])
    );
    assert!(drifted_provider.get("configuration_error").is_none());
    assert!(
        drifted_provider["guidance"]
            .as_str()
            .expect("guidance")
            .contains("PreToolUse")
    );

    fs::write(&fixture.settings_path, b"{").expect("malformed settings");
    let malformed = fixture.doctor(&fixture.agent_hook);
    assert_configuration_error(&malformed, "agent-hook-setup-output-invalid");

    fs::write(&fixture.settings_path, &original_settings).expect("restore valid settings");
    fs::set_permissions(&fixture.settings_path, fs::Permissions::from_mode(0o0))
        .expect("make settings unreadable");
    let unreadable = fixture.doctor(&fixture.agent_hook);
    fs::set_permissions(&fixture.settings_path, fs::Permissions::from_mode(0o600))
        .expect("restore settings permissions");
    assert_configuration_error(&unreadable, "agent-hook-setup-output-invalid");

    fs::remove_file(&fixture.settings_path).expect("remove settings");
    let absent = fixture.doctor(&fixture.agent_hook);
    assert_eq!(absent.code, 0, "stderr={}", absent.stderr_text());
    let absent_provider = claude_provider(&absent);
    assert_eq!(absent_provider["configured"], false);
    assert!(absent_provider.get("configuration_error").is_none());
    let mut missing_events = absent_provider["missing_owned_events"]
        .as_array()
        .expect("missing event list")
        .iter()
        .map(|event| event.as_str().expect("event name"))
        .collect::<Vec<_>>();
    missing_events.sort_unstable();
    assert_eq!(missing_events, ["PreToolUse", "UserPromptSubmit"]);

    let unavailable_hook = fixture.root.join("missing-agent-hook");
    let unavailable = fixture.doctor(&unavailable_hook);
    assert_configuration_error(&unavailable, "agent-hook-setup-unavailable");
}
