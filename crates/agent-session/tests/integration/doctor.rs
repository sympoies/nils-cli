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
