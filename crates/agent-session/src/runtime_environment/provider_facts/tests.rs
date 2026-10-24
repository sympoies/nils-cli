use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

fn record(agent: &str, args: &[&str]) -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version":crate::SESSION_DOCUMENT_VERSION, "id":"facts-fixture", "agent":agent,
        "mode":"interactive", "title":null, "cwd":"/workspace/project", "tmux_session":"facts-fixture",
        "prompt_file":null, "log_file":null, "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
        "agent_args":args,
        "runtime":{"kind":"tmux", "tmux_session":"facts-fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"first-runtime"}
    })).unwrap()
}

#[test]
fn runtime_facts_validate_capacity_and_selected_codex_provider() {
    for invalid in [
        "",
        "0",
        "-1",
        "+1",
        " 131072",
        "1.5",
        "9007199254740992",
        "18446744073709551616",
    ] {
        assert_eq!(capacity(invalid), None, "{invalid}");
    }
    assert_eq!(capacity("131072"), Some(131072));
    for (args, applicable) in [
        (vec!["--oss"], false),
        (
            vec![
                "-c",
                "model_provider=local",
                "-c",
                "model_providers.local.requires_openai_auth=false",
            ],
            false,
        ),
        (vec!["--", "--oss"], true),
        (
            vec![
                "-c",
                "model_provider='local'",
                "--config=model_providers.local.requires_openai_auth=false",
            ],
            false,
        ),
        (
            vec![
                "-cmodel_provider='local'",
                "-cmodel_providers.local.requires_openai_auth=false",
                "-cmodel_providers.local.requires_openai_auth=true",
            ],
            true,
        ),
        (
            vec!["-c", "model_providers.other.requires_openai_auth=false"],
            true,
        ),
        (vec!["--model", "custom-local-model"], true),
        (vec!["--config=not toml"], true),
    ] {
        assert_eq!(
            account_applicable(&record("codex", &args)),
            applicable,
            "{args:?}"
        );
    }
}

#[test]
fn runtime_facts_preserve_official_claude_endpoint_account_applicability() {
    for endpoint in ["https://api.anthropic.com", "https://api.anthropic.com/"] {
        let settings = json!({"env":{"ANTHROPIC_BASE_URL":endpoint}}).to_string();
        assert!(
            account_applicable(&record("claude", &["--settings", &settings])),
            "an explicit default endpoint still uses the broker account"
        );
    }
    for key in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        let settings = json!({"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com",(key):"synthetic-auth-canary"}}).to_string();
        assert!(
            !account_applicable(&record("claude", &["--settings", &settings])),
            "direct authentication bypasses the broker even on the official endpoint"
        );
    }
}

#[test]
fn runtime_facts_snapshot_environment_and_refresh_for_new_runtime() {
    let lock = GlobalStateLock::new();
    let _clear: Vec<_> = CLAUDE_KEYS
        .iter()
        .map(|key| EnvGuard::remove(&lock, key))
        .collect();
    let _capacity = EnvGuard::set(&lock, CLAUDE_KEYS[0], "65536");
    let _endpoint = EnvGuard::set(&lock, CLAUDE_KEYS[1], "http://127.0.0.1:9/private-canary");
    let mut record = record("claude", &[]);
    super::super::store_launch(&mut record);
    assert_eq!(claude_window(&record), Some(65536));
    assert!(!account_applicable(&record));
    let facts = record.extra[KEY].to_string();
    assert!(!facts.contains("private-canary"));
    assert!(!facts.contains("ANTHROPIC"));
    assert!(!facts.contains("CLAUDE_CODE"));
    let _new_capacity = EnvGuard::set(&lock, CLAUDE_KEYS[0], "262144");
    let _new_endpoint = EnvGuard::set(&lock, CLAUDE_KEYS[1], "");
    assert_eq!(
        claude_window(&record),
        Some(65536),
        "reads must not follow current daemon env"
    );
    assert!(!account_applicable(&record));
    record.runtime.as_mut().unwrap().launch_id = "next-runtime".into();
    assert_eq!(
        claude_window(&record),
        None,
        "old runtime facts cannot follow a resume"
    );
    assert!(account_applicable(&record));
    capture(&mut record);
    assert_eq!(claude_window(&record), Some(262144));
    assert!(account_applicable(&record));
    record.agent = "codex".into();
    assert_eq!(claude_window(&record), None, "facts are provider-bound");
}

#[test]
fn runtime_facts_inline_settings_override_ambient_without_exporting_values() {
    let lock = GlobalStateLock::new();
    let _clear: Vec<_> = CLAUDE_KEYS
        .iter()
        .map(|key| EnvGuard::remove(&lock, key))
        .collect();
    let _capacity = EnvGuard::set(&lock, CLAUDE_KEYS[0], "65536");
    let _endpoint = EnvGuard::set(&lock, CLAUDE_KEYS[1], "http://127.0.0.1:9/private-canary");
    let _auth = EnvGuard::set(&lock, CLAUDE_KEYS[3], "synthetic-auth-canary");
    let mut record = record(
        "claude",
        &[
            r#"--settings={"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"131072","ANTHROPIC_BASE_URL":"","ANTHROPIC_AUTH_TOKEN":""}}"#,
        ],
    );
    capture(&mut record);
    assert_eq!(claude_window(&record), Some(131072));
    assert!(account_applicable(&record));
    assert!(!record.extra[KEY].to_string().contains("canary"));
    record.agent_args = vec![
        "--settings".into(),
        json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":131072}}).to_string(),
    ];
    capture(&mut record);
    assert_eq!(
        claude_window(&record),
        None,
        "invalid explicit capacity must not reuse ambient capacity"
    );
}

#[test]
fn runtime_facts_match_claude_environment_in_existing_tmux_server() {
    let lock = GlobalStateLock::new();
    let _clear: Vec<_> = CLAUDE_KEYS
        .iter()
        .map(|key| EnvGuard::remove(&lock, key))
        .collect();
    let _capacity = EnvGuard::set(&lock, CLAUDE_KEYS[0], "131072");
    let Some(tmux) = crate::binary_on_path("tmux") else {
        eprintln!("tmux unavailable; skipping isolated environment fixture");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    struct Server {
        tmux: std::path::PathBuf,
        socket: std::path::PathBuf,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = Command::new(&self.tmux)
                .arg("-S")
                .arg(&self.socket)
                .arg("kill-server")
                .output();
        }
    }
    let server = Server {
        tmux: tmux.clone(),
        socket: tmp.path().join("tmux.sock"),
    };
    assert!(
        Command::new(&tmux)
            .args(["-f", "/dev/null", "-S"])
            .arg(&server.socket)
            .args(["new-session", "-d", "-s", "seed", "sleep", "10"])
            .env(CLAUDE_KEYS[0], "65536")
            .env(CLAUDE_KEYS[1], "http://127.0.0.1:9/old-endpoint-canary")
            .env(CLAUDE_KEYS[2], "synthetic-old-api-canary")
            .env(CLAUDE_KEYS[3], "synthetic-old-auth-canary")
            .output()
            .unwrap()
            .status
            .success()
    );
    for direct_auth in [false, true] {
        let endpoint = if direct_auth {
            "http://127.0.0.1:9/current-canary"
        } else {
            ""
        };
        let api = if direct_auth {
            "synthetic-api-canary"
        } else {
            ""
        };
        let auth = if direct_auth {
            "synthetic-'auth-$(false)-canary"
        } else {
            ""
        };
        let _endpoint = EnvGuard::set(&lock, CLAUDE_KEYS[1], endpoint);
        let _api = EnvGuard::set(&lock, CLAUDE_KEYS[2], api);
        let _auth = EnvGuard::set(&lock, CLAUDE_KEYS[3], auth);
        let mut record = record("claude", &[]);
        capture(&mut record);
        fs::create_dir_all(tmp.path().join("sessions").join(&record.id)).unwrap();
        let gate = tmp.path().join("gate");
        let broker_gate = tmp.path().join("broker-gate");
        fs::write(&gate, "").unwrap();
        fs::write(&broker_gate, "").unwrap();
        let output = tmp.path().join(format!("capacity-{direct_auth}"));
        let mut command = Command::new(&tmux);
        command
            .arg("-S")
            .arg(&server.socket)
            .args(["new-session", "-d", "-s"])
            .arg(format!("facts-{direct_auth}"));
        crate::add_runtime_tmux_environment(&mut command, tmp.path(), &record).unwrap();
        command
            .args(["sh", "-c", crate::HELD_LAUNCH_SCRIPT, "fixture"])
            .arg(&gate)
            .arg(&broker_gate)
            .arg(tmp.path().join("heartbeat"))
            .arg(tmp.path().join("capability"))
            .args(["first-runtime", "1", "/usr/bin/true"]);
        command
            .args([
                "sh",
                "-c",
                "printf '%s:%s:%s:%s' \"$CLAUDE_CODE_MAX_CONTEXT_TOKENS\" \"$ANTHROPIC_BASE_URL\" \"$ANTHROPIC_API_KEY\" \"$ANTHROPIC_AUTH_TOKEN\" > \"$1\"; exec sleep 10",
                "fixture"
            ])
            .arg(&output);
        assert!(
            crate::runtime_environment::output(command, Duration::from_secs(5))
                .unwrap()
                .status
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !output.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let observed = fs::read_to_string(output).unwrap();
        assert_eq!(
            observed,
            format!(
                "{}:{endpoint}:{api}:{auth}",
                claude_window(&record).unwrap()
            ),
            "reported capacity and account applicability must match the child, including clearing stale tmux values"
        );
        assert_eq!(account_applicable(&record), !direct_auth);
        assert!(
            fs::read_dir(tmp.path().join("sessions").join(&record.id))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".provider-env-"))
        );
    }
}

#[test]
fn runtime_facts_reject_changed_launch_configuration_and_profile() {
    let lock = GlobalStateLock::new();
    let _clear: Vec<_> = CLAUDE_KEYS
        .iter()
        .map(|key| EnvGuard::remove(&lock, key))
        .collect();
    let _capacity = EnvGuard::set(&lock, CLAUDE_KEYS[0], "131072");
    let _endpoint = EnvGuard::set(&lock, CLAUDE_KEYS[1], "http://127.0.0.1:9/private-canary");
    let mut original = record("claude", &[]);
    capture(&mut original);
    assert_eq!(claude_window(&original), Some(131072));
    assert!(!account_applicable(&original));
    for key in ["agent_profile", "agent_profile_provider_config_dir"] {
        let mut changed = original.clone();
        changed
            .runtime
            .as_mut()
            .unwrap()
            .extra
            .insert(key.into(), json!("/fixture/changed"));
        assert_eq!(claude_window(&changed), None, "{key}");
        assert!(
            account_applicable(&changed),
            "stale applicability rejected: {key}"
        );
        capture(&mut changed);
        assert_eq!(claude_window(&changed), Some(131072));
        assert!(!account_applicable(&changed));
    }
    let mut changed = original.clone();
    changed.agent_args = vec![
        "--settings".into(),
        json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"65536", "ANTHROPIC_BASE_URL":""}})
            .to_string(),
    ];
    assert_eq!(claude_window(&changed), Some(65536));
    assert!(account_applicable(&changed));
    changed.agent_args = vec![
        "--settings".into(),
        json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":131072}}).to_string(),
    ];
    assert_eq!(claude_window(&changed), None);
    // Launch facts are immutable until an explicit new launch captures again.
    assert_eq!(claude_window(&original), Some(131072));
}

#[test]
fn runtime_environment_hides_auth_values_from_tmux_arguments() {
    let lock = GlobalStateLock::new();
    let _api = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "synthetic-api-canary");
    let _auth = EnvGuard::set(&lock, "ANTHROPIC_AUTH_TOKEN", "synthetic-auth-canary");
    let tmp = tempfile::tempdir().unwrap();
    let record = record("claude", &[]);
    let mut command = Command::new("tmux");
    crate::add_runtime_tmux_environment(&mut command, tmp.path(), &record).unwrap();
    for arg in command.get_args() {
        assert!(
            !arg.to_string_lossy().contains("synthetic-api-canary"),
            "API key leaked into process arguments"
        );
        assert!(
            !arg.to_string_lossy().contains("synthetic-auth-canary"),
            "auth token leaked into process arguments"
        );
    }
}
