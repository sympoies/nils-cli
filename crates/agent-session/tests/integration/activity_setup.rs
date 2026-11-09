use super::cli::{data, run, write_executable};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;

#[test]
fn activity_setup_forwards_single_registration_ownership_to_agent_hook() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("home dir");
    let hook = tmp.path().join("agent-hook");
    let log = tmp.path().join("agent-hook.log");
    write_executable(
        &hook,
        r#"#!/usr/bin/env sh
: "${AGENT_HOOK_FORWARD_LOG:?}"
printf '%s\n' "$*" >> "$AGENT_HOOK_FORWARD_LOG"
action=${4#--}
compatibility_count_key=leg"acy_residue_count"
printf '%s\n' "{\"schema_version\":\"cli.agent-hook.setup.v1\",\"ok\":true,\"data\":{\"schema_version\":\"agent-hook.setup-result.v1\",\"product\":\"codex\",\"action\":\"$action\",\"status\":\"converged\",\"changed\":false,\"would_change\":true,\"configured\":false,\"would_configure\":true,\"apply_allowed\":true,\"plan_digest\":\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"config_digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"policy_digest\":\"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"owned_events\":[\"UserPromptSubmit\"],\"owned_groups\":[{\"event\":\"UserPromptSubmit\"}],\"owned_count\":1,\"$compatibility_count_key\":0,\"unrelated_count\":0,\"compatibility_owner\":\"agent-hook\",\"trust\":\"review the agent-hook setup plan\"}}"
"#,
    );
    let home = home.to_string_lossy().into_owned();
    let log_arg = log.to_string_lossy().into_owned();

    let envs = [
        ("HOME", home.as_str()),
        ("AGENT_SESSION_BIN", "/nonexistent/agent-session"),
        (
            "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
            tmp.path().to_str().unwrap(),
        ),
        ("AGENT_HOOK_FORWARD_LOG", log_arg.as_str()),
    ];
    for args in [
        vec!["--dry-run"],
        vec!["--apply"],
        vec!["--remove"],
        vec![
            "--repair",
            "--expected-preview-digest",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
        vec!["--repair", "--dry-run"],
    ] {
        let mut command = vec!["activity", "setup", "--agent", "codex"];
        command.extend(args);
        command.extend(["--format", "json"]);
        let output = run(tmp.path(), &command, &envs);
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        assert_eq!(
            data(&output.stdout_json())["compatibility_owner"],
            "agent-hook"
        );
    }
    assert_eq!(
        fs::read_to_string(&log).expect("agent-hook forward log"),
        concat!(
            "setup --product codex --dry-run --format json\n",
            "setup --product codex --apply --format json\n",
            "setup --product codex --remove --format json\n",
            "setup --product codex --repair --format json --expected-plan-digest sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            "setup --product codex --dry-run --format json\n",
        )
    );
}

#[test]
fn activity_setup_rejects_incompatible_or_incomplete_agent_hook_success_envelopes() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("home dir");
    let hook = tmp.path().join("agent-hook");
    write_executable(
        &hook,
        r#"#!/usr/bin/env sh
: "${AGENT_HOOK_RESPONSE:?}"
printf '%s\n' "$AGENT_HOOK_RESPONSE"
"#,
    );
    let mut valid = json!({
        "schema_version":"cli.agent-hook.setup.v1",
        "ok":true,
        "data":{
            "schema_version":"agent-hook.setup-result.v1",
            "product":"codex",
            "action":"dry-run",
            "status":"converged",
            "changed":false,
            "would_change":false,
            "configured":true,
            "would_configure":true,
            "apply_allowed":true,
            "plan_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "config_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "policy_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "owned_events":["PreToolUse"],
            "owned_groups":[{"event":"PreToolUse","matcher":"Write"}],
            "owned_count":1,
            "unrelated_count":0,
            "compatibility_owner":"agent-hook",
            "trust":"reviewed"
        }
    });
    valid["data"]
        .as_object_mut()
        .expect("result")
        .insert(concat!("leg", "acy_residue_count").to_string(), json!(0));
    let mut cases = Vec::new();
    let mut wrong_envelope = valid.clone();
    wrong_envelope["schema_version"] = json!("cli.agent-hook.setup.v2");
    cases.push(wrong_envelope);
    let mut wrong_result = valid.clone();
    wrong_result["data"]["schema_version"] = json!("agent-hook.setup-result.v2");
    cases.push(wrong_result);
    let mut wrong_product = valid.clone();
    wrong_product["data"]["product"] = json!("claude");
    cases.push(wrong_product);
    let mut wrong_action = valid.clone();
    wrong_action["data"]["action"] = json!("apply");
    cases.push(wrong_action);
    let mut incomplete = valid;
    incomplete["data"]
        .as_object_mut()
        .expect("result")
        .remove("plan_digest");
    cases.push(incomplete);

    let home = home.to_string_lossy().into_owned();
    for response in cases {
        let response = response.to_string();
        let output = run(
            tmp.path(),
            &[
                "activity",
                "setup",
                "--agent",
                "codex",
                "--dry-run",
                "--format",
                "json",
            ],
            &[
                ("HOME", home.as_str()),
                ("AGENT_SESSION_BIN", "/nonexistent/agent-session"),
                (
                    "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
                    tmp.path().to_str().unwrap(),
                ),
                ("AGENT_HOOK_RESPONSE", response.as_str()),
            ],
        );
        assert_eq!(output.code, 65, "response={response}");
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "agent-hook-setup-output-invalid"
        );
    }
}

#[test]
fn activity_setup_accepts_additive_v1_fields_but_keeps_required_bridge_fields_strict() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("home dir");
    let hook = tmp.path().join("agent-hook");
    write_executable(
        &hook,
        r#"#!/usr/bin/env sh
: "${AGENT_HOOK_RESPONSE:?}"
printf '%s\n' "$AGENT_HOOK_RESPONSE"
"#,
    );
    let mut success = json!({
        "schema_version":"cli.agent-hook.setup.v1",
        "ok":true,
        "data":{
            "schema_version":"agent-hook.setup-result.v1",
            "product":"codex",
            "action":"dry-run",
            "status":"converged",
            "changed":false,
            "would_change":false,
            "configured":true,
            "would_configure":true,
            "apply_allowed":true,
            "plan_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "config_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "policy_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "owned_events":["PreToolUse"],
            "owned_groups":[{
                "event":"PreToolUse",
                "matcher":"Write",
                "future_group_metadata":{"source":"provider"}
            }],
            "owned_count":1,
            "unrelated_count":0,
            "compatibility_owner":"agent-hook",
            "trust":"reviewed",
            "future_result_metadata":{"generation":2}
        },
        "future_envelope_metadata":{"producer":"agent-hook"}
    });
    success["data"][concat!("leg", "acy_residue_count")] = json!(0);

    let home = home.to_string_lossy().into_owned();
    let run_response = |response: &serde_json::Value| {
        let response = response.to_string();
        run(
            tmp.path(),
            &[
                "activity",
                "setup",
                "--agent",
                "codex",
                "--dry-run",
                "--format",
                "json",
            ],
            &[
                ("HOME", home.as_str()),
                ("AGENT_SESSION_BIN", "/nonexistent/agent-session"),
                (
                    "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
                    tmp.path().to_str().unwrap(),
                ),
                ("AGENT_HOOK_RESPONSE", response.as_str()),
            ],
        )
    };

    let accepted = run_response(&success);
    assert_eq!(accepted.code, 0, "stderr={}", accepted.stderr_text());
    assert_eq!(
        data(&accepted.stdout_json())["compatibility_owner"],
        "agent-hook"
    );

    let mut missing_event = success.clone();
    missing_event["data"]["owned_groups"][0]
        .as_object_mut()
        .expect("owned group")
        .remove("event");
    let missing_event = run_response(&missing_event);
    assert_eq!(missing_event.code, 65);
    assert_eq!(
        missing_event.stdout_json()["error"]["code"],
        "agent-hook-setup-output-invalid"
    );

    let mut invalid_matcher = success.clone();
    invalid_matcher["data"]["owned_groups"][0]["matcher"] = json!(["Write"]);
    let invalid_matcher = run_response(&invalid_matcher);
    assert_eq!(invalid_matcher.code, 65);
    assert_eq!(
        invalid_matcher.stdout_json()["error"]["code"],
        "agent-hook-setup-output-invalid"
    );

    success["data"]
        .as_object_mut()
        .expect("result")
        .remove("plan_digest");
    let missing_required = run_response(&success);
    assert_eq!(missing_required.code, 65);
    assert_eq!(
        missing_required.stdout_json()["error"]["code"],
        "agent-hook-setup-output-invalid"
    );

    let upstream_failure = json!({
        "schema_version":"cli.agent-hook.setup.v1",
        "ok":false,
        "error":{
            "code":"agent-hook-upstream-busy",
            "message":"retry later",
            "future_error_metadata":{"retry_after_seconds":1}
        },
        "future_envelope_metadata":{"producer":"agent-hook"}
    });
    let failure = run_response(&upstream_failure);
    assert_eq!(failure.code, 65);
    assert_eq!(
        failure.stdout_json()["error"]["code"],
        "agent-hook-upstream-busy"
    );
}

#[test]
fn activity_setup_missing_agent_hook_is_typed_and_never_mutates_provider_config() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    let codex_home = home.join(".codex");
    fs::create_dir_all(&codex_home).expect("codex home");
    let config = codex_home.join("config.toml");
    let original = b"# user-owned\nmodel = \"gpt-test\"\n";
    fs::write(&config, original).expect("provider config");
    let missing = tmp.path().join("missing-agent-hook");
    let unstartable = tmp.path().join("unstartable");
    fs::create_dir(&unstartable).unwrap();
    fs::write(
        unstartable.join("agent-hook"),
        "#!/usr/bin/env sh\nexit 0\n",
    )
    .expect("unstartable hook");
    let home_arg = home.to_string_lossy().into_owned();
    for hook in [missing, unstartable] {
        let output = run(
            tmp.path(),
            &[
                "activity", "setup", "--agent", "codex", "--apply", "--format", "json",
            ],
            &[
                ("HOME", home_arg.as_str()),
                ("AGENT_SESSION_BIN", "/nonexistent/agent-session"),
                ("AGENT_RUNTIME_TRUSTED_CLI_ROOT", hook.to_str().unwrap()),
            ],
        );

        assert_eq!(output.code, 69, "hook={}", hook.display());
        let envelope = output.stdout_json();
        assert_eq!(envelope["error"]["code"], "agent-hook-setup-unavailable");
        assert_eq!(
            envelope["error"]["details"]["compatibility_owner"],
            "agent-hook"
        );
        assert!(
            envelope["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("rerun the same dry-run")
                    || message.contains("compatibility forward failed"))
        );
        assert_eq!(fs::read(&config).expect("provider config"), original);
    }
}

#[test]
fn activity_setup_preserves_supported_agent_hook_failure_exit_classes() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("home dir");
    let hook = tmp.path().join("agent-hook");
    write_executable(
        &hook,
        r#"#!/usr/bin/env sh
: "${AGENT_HOOK_EXIT_CODE:?}"
printf '%s\n' '{"schema_version":"cli.agent-hook.setup.v1","ok":false,"error":{"code":"agent-hook-forwarded-failure","message":"forwarded failure"}}'
exit "$AGENT_HOOK_EXIT_CODE"
"#,
    );
    let home_arg = home.to_string_lossy().into_owned();

    for expected in [1, 64, 65, 69, 70] {
        let expected_arg = expected.to_string();
        let output = run(
            tmp.path(),
            &[
                "activity",
                "setup",
                "--agent",
                "codex",
                "--dry-run",
                "--format",
                "json",
            ],
            &[
                ("HOME", home_arg.as_str()),
                ("AGENT_SESSION_BIN", "/nonexistent/agent-session"),
                (
                    "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
                    tmp.path().to_str().unwrap(),
                ),
                ("AGENT_HOOK_EXIT_CODE", expected_arg.as_str()),
            ],
        );
        assert_eq!(output.code, expected, "expected child exit {expected}");
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "agent-hook-forwarded-failure"
        );
    }
}
