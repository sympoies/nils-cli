mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn generated_entrypoints_never_fall_back_to_path() {
    let fixture = Fixture::new(POLICY);
    let selected = fixture.root.join("selected");
    let poison = fixture.root.join("poison");
    let empty = fixture.root.join("empty");
    let stale = fixture.root.join("stale");
    for directory in [&selected, &poison, &empty, &stale] {
        fs::create_dir_all(directory).unwrap();
    }
    for name in ["agent-hook", nils_common::session_binary::NAME] {
        executable(
            &poison.join(name),
            "#!/bin/sh\nprintf poison > \"$POISON\"\n",
        );
        executable(
            &selected.join(name),
            "#!/bin/sh\nprintf selected > \"$RECORD\"\n",
        );
    }
    let mut commands = Vec::new();
    for product in ["codex", "claude"] {
        let result = fixture.run(
            &["setup", "--product", product, "--apply", "--format", "json"],
            None,
        );
        assert_eq!(result.code, 0, "{}", result.stderr_text());
        let command = if product == "codex" {
            let config: toml::Value = toml::from_str(
                &fs::read_to_string(fixture.home.join(".codex/config.toml")).unwrap(),
            )
            .unwrap();
            let notify = config["notify"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            commands.push(notify);
            config["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            let config: serde_json::Value = serde_json::from_slice(
                &fs::read(fixture.home.join(".claude/settings.json")).unwrap(),
            )
            .unwrap();
            config["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        commands.push(vec!["/bin/sh".into(), "-c".into(), command]);
    }
    let marker = fixture.root.join("poison-ran");
    let record = fixture.root.join("selected-ran");
    let path = format!("{}:{}:/usr/bin:/bin", poison.display(), selected.display());
    let pin = stale.join(nils_common::session_binary::NAME);
    for command in &commands {
        let run = |root: &Path| {
            Command::new(&command[0])
                .args(&command[1..])
                .env("PATH", &path)
                .env("AGENT_SESSION_BIN", &pin)
                .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", root)
                .env("POISON", &marker)
                .env("RECORD", &record)
                .stdin(Stdio::null())
                .status()
                .unwrap()
        };
        fs::write(&record, "untouched").unwrap();
        assert_eq!(run(&empty).code(), Some(127));
        assert_eq!(fs::read_to_string(&record).unwrap(), "untouched");
        assert!(!marker.exists());
        assert!(run(&selected).success());
        assert_eq!(fs::read_to_string(&record).unwrap(), "selected");
        assert!(!marker.exists());
    }
    fs::write(&pin, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&pin, fs::Permissions::from_mode(0o644)).unwrap();
    for command in &commands {
        fs::write(&record, "untouched").unwrap();
        let status = Command::new(&command[0])
            .args(&command[1..])
            .env("PATH", &path)
            .env("AGENT_SESSION_BIN", &pin)
            .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", &selected)
            .env("POISON", &marker)
            .env("RECORD", &record)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read_to_string(&record).unwrap(), "selected");
        assert!(!marker.exists());
    }
}
