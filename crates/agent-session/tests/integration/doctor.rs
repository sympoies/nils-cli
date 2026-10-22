use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write executable");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable");
}

#[test]
fn codex_activity_doctor_recognizes_audited_computer_use_owned_notify() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).expect("Codex directory");
    fs::write(codex.join("hooks.json"), r#"{"hooks":{}}"#).expect("hooks config");
    let helper = codex.join(
        "computer-use/Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
    );
    fs::create_dir_all(helper.parent().expect("Computer Use helper parent"))
        .expect("Computer Use helper directory");
    write_executable(&helper, "#!/usr/bin/env sh\nexit 0\n");
    let owned = ["agent-session", "activity", "notify", "--agent", "codex"];
    let notify = [
        helper.to_string_lossy().into_owned(),
        "turn-ended".to_string(),
        "--previous-notify".to_string(),
        serde_json::to_string(&owned).expect("owned notify JSON"),
    ];
    let mut document = toml_edit::DocumentMut::new();
    let mut array = toml_edit::Array::new();
    array.extend(notify.iter().map(String::as_str));
    document["notify"] = toml_edit::value(array);
    fs::write(codex.join("config.toml"), document.to_string()).expect("Codex config");
    let home_arg = home.to_string_lossy().to_string();
    let codex_arg = codex.to_string_lossy().to_string();
    let args = ["activity", "doctor", "--agent", "codex", "--format", "json"];
    let envs = [
        ("HOME", home_arg.as_str()),
        ("CODEX_HOME", codex_arg.as_str()),
    ];

    let doctor = super::cli::run(tmp.path(), &args, &envs);
    assert_eq!(doctor.code, 0, "stderr={}", doctor.stderr_text());
    assert_eq!(
        super::cli::data(&doctor.stdout_json())["providers"][0]["notification_mode"],
        "composed"
    );

    fs::remove_file(&helper).expect("remove Computer Use helper");
    let unavailable = super::cli::run(tmp.path(), &args, &envs);
    assert_eq!(unavailable.code, 0, "stderr={}", unavailable.stderr_text());
    assert_eq!(
        super::cli::data(&unavailable.stdout_json())["providers"][0]["notification_mode"],
        "conflict"
    );
}

#[test]
fn codex_activity_doctor_surfaces_notification_config_errors() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).expect("codex dir");
    fs::write(codex.join("hooks.json"), r#"{"hooks":{}}"#).expect("hooks config");
    fs::write(codex.join("config.toml"), b"notify = [\"unterminated\"")
        .expect("malformed notify config");
    let home_arg = home.to_string_lossy().to_string();
    let codex_arg = codex.to_string_lossy().to_string();
    let doctor = super::cli::run(
        tmp.path(),
        &["activity", "doctor", "--agent", "codex", "--format", "json"],
        &[
            ("HOME", home_arg.as_str()),
            ("CODEX_HOME", codex_arg.as_str()),
        ],
    );
    assert_eq!(doctor.code, 0, "stderr={}", doctor.stderr_text());
    let doctor_json = doctor.stdout_json();
    let provider = &super::cli::data(&doctor_json)["providers"][0];
    assert_eq!(provider["configured"], false);
    assert_eq!(provider["configuration_error"], "provider-config-invalid");
    assert_eq!(provider["notification_mode"], "invalid");
    assert!(
        provider["guidance"]
            .as_str()
            .expect("guidance")
            .contains("fix the provider configuration before running repair")
    );
}

#[test]
fn activity_doctor_lists_running_sessions_with_removed_pinned_binaries() {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let (tmux, log) = super::cli::fake_tmux(tmp.path());
    let helper = tmp.path().join("removed-agent-session");
    write_executable(&helper, "#!/bin/sh\nexit 0\n");
    let session = state.join("sessions/pinned-fixture");
    fs::create_dir_all(&session).unwrap();
    let record = json!({
        "schema_version": "agent-session.session.v1", "id": "pinned-fixture",
        "agent": "codex", "mode": "interactive", "title": null,
        "cwd": tmp.path(), "tmux_session": "pinned-fixture",
        "prompt_file": null, "log_file": null,
        "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z"
    });
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    // tmux records the pinned helper in the session environment at launch.
    fs::write(
        format!("{}.environments", log.display()),
        format!("pinned-fixture\tAGENT_SESSION_BIN\t{}\n", helper.display()),
    )
    .unwrap();
    let envs = [
        ("HOME", home.to_str().unwrap()),
        ("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap()),
        ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
    ];
    let args = [
        "--state-dir",
        state.to_str().unwrap(),
        "activity",
        "doctor",
        "--format",
        "json",
    ];
    let run = |args: &[&str], extra: &[(&str, &str)]| {
        let mut envs = envs.to_vec();
        envs.extend_from_slice(extra);
        let output = super::cli::run(tmp.path(), args, &envs);
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        output
    };
    fs::remove_file(&helper).unwrap();
    let removed = run(&args, &[]).stdout_json();
    assert_eq!(
        super::cli::data(&removed)["missing_pinned_binaries"],
        json!([{
            "session_id": "pinned-fixture", "binary_path": helper,
            "code": "pinned-agent-session-binary-missing"
        }])
    );
    let mut text_args = args;
    text_args[5] = "text";
    let text = run(&text_args, &[]).stdout_text();
    assert!(text.contains("pinned-fixture"));
    assert!(text.contains("pinned-agent-session-binary-missing"));
    assert!(text.contains(helper.to_str().unwrap()));
    // Stopped sessions and helpers that still exist are not stale findings.
    let stopped = run(&args, &[("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0")]).stdout_json();
    assert_eq!(
        super::cli::data(&stopped)["missing_pinned_binaries"],
        json!([])
    );
    write_executable(&helper, "#!/bin/sh\nexit 0\n");
    let present = run(&args, &[]).stdout_json();
    assert_eq!(
        super::cli::data(&present)["missing_pinned_binaries"],
        json!([])
    );
}
