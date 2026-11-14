mod support;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use support::Fixture;
const PERMISSION_POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.11.14.1"
[[rules]]
id = "session.activity"
products = ["claude"]
events = ["PermissionRequest", "PreToolUse"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;
#[test]
fn permission_hook_setup_refreshes_stale_timeout_and_reports_drift() {
    let fixture = Fixture::new(PERMISSION_POLICY);
    let apply = || {
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
        assert_eq!(preview.code, 0, "{}", preview.stderr_text());
        let digest = preview.stdout_json()["data"]["plan_digest"]
            .as_str()
            .unwrap()
            .to_string();
        let result = fixture.run(
            &[
                "setup",
                "--product",
                "claude",
                "--apply",
                "--expected-plan-digest",
                &digest,
                "--format",
                "json",
            ],
            None,
        );
        assert_eq!(result.code, 0, "{}", result.stderr_text());
    };
    apply();
    let path = fixture.home.join(".claude/settings.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        config["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
        600
    );
    assert_eq!(config["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 60);
    config["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(60);
    fs::write(&path, config.to_string()).unwrap();
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
    assert_eq!(preview.stdout_json()["data"]["status"], "drifted");
    apply();
    let config: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(
        config["hooks"]["PermissionRequest"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        config["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
        600
    );
}
