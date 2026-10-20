mod support;

use std::fs;

use pretty_assertions::assert_eq;
use serde_json::Value;

use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "fixture-claude-config-path"
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

#[test]
fn claude_setup_writes_settings_to_the_absolute_config_dir_override() {
    let fixture = Fixture::new(POLICY);
    let config_dir = fixture.root.join("claude-account-config");
    fs::create_dir_all(&config_dir).expect("Claude config directory");
    let config_dir_arg = config_dir.to_str().expect("config directory path");

    let applied = fixture.run_with_env(
        &[
            "setup",
            "--product",
            "claude",
            "--apply",
            "--format",
            "json",
        ],
        None,
        &[("CLAUDE_CONFIG_DIR", config_dir_arg)],
    );
    assert_eq!(
        applied.code,
        0,
        "stdout={} stderr={}",
        applied.stdout_text(),
        applied.stderr_text()
    );

    let settings = config_dir.join("settings.json");
    let document: Value = serde_json::from_slice(&fs::read(&settings).expect("override settings"))
        .expect("settings JSON");
    assert_eq!(
        document["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        "agent-hook dispatch --product claude"
    );
    assert!(
        !fixture.home.join(".claude/settings.json").exists(),
        "setup must use the configured Claude settings path"
    );
}
