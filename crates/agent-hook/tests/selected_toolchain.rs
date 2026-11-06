mod support;

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "fixture"
version = "1"

[[rules]]
id = "fixture.start"
products = ["codex", "claude"]
events = ["SessionStart"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "decision.allow.v1", reason_code = "fixture" }
"#;

fn provider_ingress_uses_selected_toolchain(product: &str) {
    let fixture = Fixture::new(POLICY);
    let selected = fixture.root.join("selected");
    let next = fixture.root.join("next");
    let poison = fixture.root.join("poison");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(&next).unwrap();
    fs::create_dir_all(&poison).unwrap();
    for (dir, label) in [
        (&selected, "selected"),
        (&next, "next"),
        (&poison, "poison"),
    ] {
        for name in ["agent-hook", nils_common::session_binary::NAME] {
            let path = dir.join(name);
            fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{label}:{name}' \"$@\" > \"$RECORD\"\ncat >> \"$RECORD\"\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    {
        let result = fixture.run(
            &["setup", "--product", product, "--apply", "--format", "json"],
            None,
        );
        assert_eq!(result.code, 0, "{}", result.stderr_text());
        let config = if product == "codex" {
            let doc: toml::Value = toml::from_str(
                &fs::read_to_string(fixture.home.join(".codex/config.toml")).unwrap(),
            )
            .unwrap();
            doc["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            let doc: serde_json::Value = serde_json::from_slice(
                &fs::read(fixture.home.join(".claude/settings.json")).unwrap(),
            )
            .unwrap();
            doc["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let record = fixture.root.join("record");
        for (dir, label) in [(&selected, "selected"), (&next, "next")] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &config])
                .env(
                    "PATH",
                    format!("{}:{}:/usr/bin:/bin", poison.display(), selected.display()),
                )
                .env(
                    "AGENT_SESSION_BIN",
                    dir.join(nils_common::session_binary::NAME),
                )
                .env("RECORD", &record)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"{\"hook_event_name\":\"SessionStart\"}")
                .unwrap();
            assert!(child.wait().unwrap().success());
            assert_eq!(
                fs::read_to_string(&record).unwrap(),
                format!(
                    "{label}:agent-hook\ndispatch\n--product\n{product}\n{{\"hook_event_name\":\"SessionStart\"}}"
                )
            );
        }
        let status = Command::new("/bin/sh")
            .args(["-c", &config])
            .env(
                "PATH",
                format!("{}:{}:/usr/bin:/bin", poison.display(), selected.display()),
            )
            .env_remove("AGENT_SESSION_BIN")
            .env(
                "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
                format!("{}:{}", next.display(), selected.display()),
            )
            .env("RECORD", &record)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(&record).unwrap(),
            format!("next:agent-hook\ndispatch\n--product\n{product}\n")
        );
    }
}

#[test]
fn codex_ingress_uses_selected_toolchain() {
    provider_ingress_uses_selected_toolchain("codex");
}

#[test]
fn claude_ingress_uses_selected_toolchain() {
    provider_ingress_uses_selected_toolchain("claude");
}

#[test]
fn codex_notify_uses_selected_toolchain() {
    let fixture = Fixture::new(POLICY);
    let selected = fixture.root.join("selected");
    let poison = fixture.root.join("poison");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(&poison).unwrap();
    for (dir, label) in [(&selected, "selected"), (&poison, "poison")] {
        let path = dir.join(nils_common::session_binary::NAME);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{label}' \"$@\" > \"$RECORD\"\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let result = fixture.run(
        &["setup", "--product", "codex", "--apply", "--format", "json"],
        None,
    );
    assert_eq!(result.code, 0, "{}", result.stderr_text());
    let doc: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.home.join(".codex/config.toml")).unwrap())
            .unwrap();
    let argv: Vec<_> = doc["notify"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let record = fixture.root.join("notify-record");
    assert!(
        Command::new(argv[0])
            .args(&argv[1..])
            .arg("payload")
            .env(
                "PATH",
                format!("{}:{}:/usr/bin:/bin", poison.display(), selected.display())
            )
            .env(
                "AGENT_SESSION_BIN",
                selected.join(nils_common::session_binary::NAME)
            )
            .env("RECORD", &record)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::read_to_string(record).unwrap(),
        "selected\nactivity\nnotify\n--agent\ncodex\npayload\n"
    );
}

#[test]
fn composed_notification_uses_selected_toolchain_and_restores_original() {
    let fixture = Fixture::new(POLICY);
    let selected = fixture.root.join("selected");
    let poison = fixture.root.join("poison");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(&poison).unwrap();
    for (dir, label) in [(&selected, "selected"), (&poison, "poison")] {
        let path = dir.join(nils_common::session_binary::NAME);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{label}' \"$@\" > \"$RECORD\"\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = fixture.home.join(".codex/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let original = b"notify = [\"/bin/true\", \"argument\"]\n";
    fs::write(&config, original).unwrap();
    Fixture::set_private(&config);
    let preview = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    let digest = preview.stdout_json()["data"]["plan_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let applied = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--apply",
            "--expected-plan-digest",
            &digest,
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(applied.code, 0, "{}", applied.stderr_text());
    let doc: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    let argv: Vec<_> = doc["notify"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let record = fixture.root.join("record");
    assert!(
        Command::new(argv[0])
            .args(&argv[1..])
            .arg("payload")
            .env(
                "PATH",
                format!("{}:{}:/usr/bin:/bin", poison.display(), selected.display())
            )
            .env(
                "AGENT_SESSION_BIN",
                selected.join(nils_common::session_binary::NAME)
            )
            .env("RECORD", &record)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::read_to_string(record).unwrap(),
        "selected\nactivity\nnotify\n--agent\ncodex\n--forward-notify-argv-json\n[\"/bin/true\",\"argument\"]\npayload\n"
    );
    let removed = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--remove",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(removed.code, 0, "{}", removed.stderr_text());
    assert_eq!(fs::read(config).unwrap(), original);
}

#[test]
fn old_notify_and_guarded_activity_are_migrated_without_touching_user_config() {
    let fixture = Fixture::new(POLICY);
    let config = fixture.home.join(".codex/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let guard = format!(
        "sh -c 'if [ \"${{AGENT_SESSION_ATTENTION_AUTHORITY:-hook}}\" = protocol ]; then exit 0; fi; exec {} activity hook --agent codex'",
        nils_common::session_binary::NAME
    );
    fs::write(&config, format!("model = \"fixture\"\nnotify = [\"{}\", \"activity\", \"notify\", \"--agent\", \"codex\"]\n[[hooks.PermissionRequest]]\n[[hooks.PermissionRequest.hooks]]\ntype = \"command\"\ncommand = {}\ntimeout = 5\n", nils_common::session_binary::NAME, toml::Value::String(guard))).unwrap();
    Fixture::set_private(&config);
    let preview = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(preview.code, 0, "{}", preview.stderr_text());
    assert_eq!(preview.stdout_json()["data"]["configured"], false);
    let digest = preview.stdout_json()["data"]["plan_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let apply = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--apply",
            "--expected-plan-digest",
            &digest,
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(apply.code, 0, "{}", apply.stderr_text());
    let after = fs::read_to_string(&config).unwrap();
    assert!(after.contains("model = \"fixture\""));
    assert!(!after.contains("sh -c 'if"));
    let document: toml::Value = toml::from_str(&after).unwrap();
    assert_eq!(document["notify"][0].as_str(), Some("/bin/sh"));
    let again = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(again.stdout_json()["data"]["would_change"], false);
}

#[test]
fn old_wrapped_composed_notify_migrates_without_losing_forwarded_command() {
    let fixture = Fixture::new(POLICY);
    let codex = fixture.home.join(".codex");
    let helper = codex.join("computer-use/Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient");
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let base = vec![helper.to_str().unwrap().to_string(), "turn-ended".into()];
    let foreign = vec!["/bin/true", "argument"];
    let mut old = [
        nils_common::session_binary::NAME,
        "activity",
        "notify",
        "--agent",
        "codex",
        "--forward-notify-argv-json",
    ]
    .map(str::to_string)
    .to_vec();
    old.push(serde_json::to_string(&foreign).unwrap());
    let mut wrapped = base.clone();
    wrapped.push("--previous-notify".into());
    wrapped.push(serde_json::to_string(&old).unwrap());
    let config = codex.join("config.toml");
    let mut document = toml_edit::DocumentMut::new();
    let mut array = toml_edit::Array::new();
    array.extend(wrapped.iter().map(String::as_str));
    document["notify"] = toml_edit::value(array);
    fs::write(&config, document.to_string()).unwrap();
    Fixture::set_private(&config);
    assert_eq!(
        agent_hook::setup::codex_notification_ownership(&config, &wrapped),
        None
    );
    let preview = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    let digest = preview.stdout_json()["data"]["plan_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let apply = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--apply",
            "--expected-plan-digest",
            &digest,
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(apply.code, 0, "{}", apply.stderr_text());
    let document: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    let previous: Vec<String> =
        serde_json::from_str(document["notify"][3].as_str().unwrap()).unwrap();
    assert_eq!(previous[0], "/bin/sh");
    assert_eq!(
        previous.last().unwrap(),
        &serde_json::to_string(&foreign).unwrap()
    );
    let large_foreign = vec!["/bin/true".to_string(), "x".repeat(14_500)];
    *old.last_mut().unwrap() = serde_json::to_string(&large_foreign).unwrap();
    *wrapped.last_mut().unwrap() = serde_json::to_string(&old).unwrap();
    let mut raw: toml_edit::DocumentMut = fs::read_to_string(&config).unwrap().parse().unwrap();
    let mut array = toml_edit::Array::new();
    array.extend(wrapped.iter().map(String::as_str));
    raw["notify"] = toml_edit::value(array);
    fs::write(&config, raw.to_string()).unwrap();
    let before = fs::read(&config).unwrap();
    let rejected = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(rejected.code, 65, "{}", rejected.stderr_text());
    assert_eq!(
        rejected.stdout_json()["error"]["code"],
        "provider-notification-config-conflict"
    );
    assert!(
        rejected.stdout_json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("migration exceeds")
    );
    assert_eq!(fs::read(&config).unwrap(), before);
}
