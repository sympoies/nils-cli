//! `agent-session serve --config <file>` replaces the JSON-in-environment
//! values deployments assemble in shell (sympoies/nils-cli#1819). These tests
//! drive the real binary through `--check`, which validates the document and
//! the environment it would merge with, then exits without serving.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::assert_eq;
use serde_json::Value;

const SCHEMA: &str = "cli.agent-session.serve-config.v1";
const EXIT_USAGE: i32 = 64;
/// Every environment input the config file can also supply, plus the token so
/// a check can never pick up an operator bearer from the test runner.
const MANAGED_ENV: &[&str] = &[
    "AGENT_SESSION_LAUNCH_PROFILES",
    "AGENT_SESSION_RETITLE_CONFIG",
    "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
    "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
    "AGENT_SESSION_TOKEN",
];

const VALID_TOML: &str = r#"
schema_version = "agent-session.serve-config.v1"

[path]
append = ["/opt/example/bin", "/opt/example/tools"]

[codex_account_broker]
argv = ["/opt/example/broker", "--mode", "serve"]

[claude_account_broker]
argv = ["/opt/example/broker", "--mode", "serve"]

[retitle]
provider = "openai_compatible"
base_url = "http://127.0.0.1:1237/v1"
model = "example-model"
api_key_env = "EXAMPLE_API_KEY"
timeout_ms = 20000

[retitle.context]
max_chars = 12000

[[launch_profiles]]
id = "dsh-alt"
label = "DSH Alt"
agent = "dsh"
agent_bin = "/opt/example/dsh"
readiness_args = ["--version"]

[[launch_profiles]]
id = "dsh-workbench"
label = "DSH"
agent = "dsh"
agent_bin = "/opt/example/workbench"
"#;

struct Fixture {
    dir: ScopedTempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            dir: ScopedTempDir::with_prefix("serve-config-"),
        };
        fs::create_dir_all(fixture.state_dir()).expect("state dir");
        fixture
    }

    fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, body).expect("write config");
        path
    }

    fn command(&self, args: &[&str], env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        command
            .arg("serve")
            .args(args)
            .env("AGENT_SESSION_STATE_DIR", self.state_dir())
            .stdin(Stdio::null());
        for key in MANAGED_ENV {
            command.env_remove(key);
        }
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }

    fn check(&self, config: &Path, env: &[(&str, &str)]) -> Output {
        let config = config.to_str().expect("utf-8 config path");
        self.command(&["--config", config, "--check", "--format", "json"], env)
            .output()
            .expect("run serve --check")
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not a JSON envelope ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A failed check: exit 64, a `cli-envelope@v1` failure, and nothing that
/// reveals where the file lives.
fn failure(fixture: &Fixture, output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(EXIT_USAGE), "{output:?}");
    let value = envelope(output);
    assert_eq!(value["schema_version"], SCHEMA);
    assert_eq!(value["ok"], false);
    let root = fixture.dir.path().to_string_lossy().into_owned();
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(!text.contains(&root), "diagnostic leaked a path: {text}");
    }
    value["error"].clone()
}

fn warnings(value: &Value) -> Vec<String> {
    value["warnings"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().expect("warning string").to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn check_accepts_a_versioned_toml_document() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);

    let output = fixture.check(&config, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = envelope(&output);
    assert_eq!(value["schema_version"], SCHEMA);
    assert_eq!(value["ok"], true);
    let data = &value["data"];
    assert_eq!(
        data["config_schema_version"],
        "agent-session.serve-config.v1"
    );
    assert_eq!(data["format"], "toml");
    assert_eq!(data["launch_profiles"]["source"], "file");
    assert_eq!(
        data["launch_profiles"]["ids"],
        serde_json::json!(["dsh-alt", "dsh-workbench"])
    );
    assert_eq!(data["retitle"]["source"], "file");
    assert_eq!(data["codex_account_broker"]["source"], "file");
    assert_eq!(data["claude_account_broker"]["source"], "file");
    assert_eq!(data["path"]["append"], 2);
    assert_eq!(warnings(&value), Vec::<String>::new());
}

#[test]
fn check_accepts_the_same_document_as_json() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.json",
        r#"{
          "schema_version": "agent-session.serve-config.v1",
          "path": {"append": ["/opt/example/bin"]},
          "codex_account_broker": {"argv": ["/opt/example/broker"]},
          "retitle": {
            "provider": "codex_subscription",
            "account_selection": "default_with_capacity",
            "codex_bin": "/opt/example/codex",
            "model": "example-model",
            "fallback": {
              "provider": "openai_compatible",
              "base_url": "http://127.0.0.1:1237/v1",
              "model": "example-local",
              "timeout_ms": 60000,
              "temperature": 0,
              "json_response": true
            }
          },
          "launch_profiles": [
            {"id": "dsh-workbench", "label": "DSH", "agent": "dsh", "agent_bin": "/opt/example/dsh"}
          ]
        }"#,
    );

    let output = fixture.check(&config, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &envelope(&output)["data"];
    assert_eq!(data["format"], "json");
    assert_eq!(data["retitle"]["source"], "file");
    assert_eq!(
        data["launch_profiles"]["ids"],
        serde_json::json!(["dsh-workbench"])
    );
}

#[test]
fn an_empty_document_with_only_its_version_is_valid() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\n",
    );

    let output = fixture.check(&config, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &envelope(&output)["data"];
    assert_eq!(data["launch_profiles"]["source"], "none");
    assert_eq!(data["retitle"]["source"], "none");
    assert_eq!(data["codex_account_broker"]["source"], "none");
    assert_eq!(data["path"]["append"], 0);
}

#[test]
fn missing_or_unsupported_schema_version_names_the_key() {
    let fixture = Fixture::new();
    for body in [
        "[path]\nappend = []\n",
        "schema_version = \"agent-session.serve-config.v2\"\n",
        "schema_version = 1\n",
    ] {
        let config = fixture.write("serve.toml", body);
        let error = failure(&fixture, &fixture.check(&config, &[]));
        assert_eq!(error["code"], "serve-config-unsupported-version", "{body}");
        assert_eq!(error["details"]["key"], "schema_version", "{body}");
    }
}

#[test]
fn unknown_keys_are_rejected_by_their_full_key_path() {
    let fixture = Fixture::new();
    for (body, key) in [
        ("retitle_config = \"{}\"\n", "retitle_config"),
        ("[path]\nprepend = [\"/opt/x\"]\n", "path.prepend"),
        (
            "[codex_account_broker]\ncommand = \"/opt/broker\"\n",
            "codex_account_broker.command",
        ),
        (
            "[[launch_profiles]]\nid = \"a\"\nlabel = \"A\"\nagent = \"codex\"\nagent_bin = \"/opt/a\"\nenv = []\n",
            "launch_profiles[0].env",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\ntimeout = 5\n",
            "retitle.timeout",
        ),
    ] {
        let config = fixture.write(
            "serve.toml",
            &format!("schema_version = \"agent-session.serve-config.v1\"\n{body}"),
        );
        let error = failure(&fixture, &fixture.check(&config, &[]));
        assert_eq!(error["code"], "serve-config-unknown-key", "{body}");
        assert_eq!(error["details"]["key"], key, "{body}");
        assert!(
            error["message"].as_str().unwrap().contains(key),
            "message must name {key}: {error}"
        );
    }
}

#[test]
fn invalid_values_name_the_offending_key() {
    let fixture = Fixture::new();
    for (body, key) in [
        (
            "[path]\nappend = [\"/opt/ok\", \"relative/bin\"]\n",
            "path.append[1]",
        ),
        ("[path]\nappend = [\"\"]\n", "path.append[0]"),
        ("[path]\nappend = [\"/opt/a:/opt/b\"]\n", "path.append[0]"),
        ("[path]\nappend = \"/opt/x\"\n", "path.append"),
        (
            "[codex_account_broker]\nargv = []\n",
            "codex_account_broker.argv",
        ),
        (
            "[[launch_profiles]]\nid = \"a\"\nlabel = \"A\"\nagent = \"codex\"\nagent_bin = \"relative/codex\"\n",
            "launch_profiles[0]",
        ),
        (
            "[[launch_profiles]]\nid = \"a\"\nlabel = \"A\"\nagent = \"codex\"\nagent_bin = \"/opt/a\"\n\n[[launch_profiles]]\nid = \"a\"\nlabel = \"B\"\nagent = \"codex\"\nagent_bin = \"/opt/b\"\n",
            "launch_profiles[1].id",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\ntimeout_ms = 5\n",
            "retitle",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.fallback]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:2/v1\"\nmodel = \"f\"\ntimeout_ms = 5\n",
            "retitle.fallback",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.fallback]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:2/v1\"\nmodel = \"f\"\nmax_concurrency = 2\n",
            "retitle.fallback.max_concurrency",
        ),
    ] {
        let config = fixture.write(
            "serve.toml",
            &format!("schema_version = \"agent-session.serve-config.v1\"\n{body}"),
        );
        let error = failure(&fixture, &fixture.check(&config, &[]));
        assert_eq!(error["code"], "serve-config-invalid-value", "{body}");
        assert_eq!(error["details"]["key"], key, "{body}");
        assert_eq!(error["details"]["source"], "file", "{body}");
    }
}

#[test]
fn path_append_is_bounded() {
    let fixture = Fixture::new();
    let entries: Vec<String> = (0..17).map(|index| format!("\"/opt/p{index}\"")).collect();
    let config = fixture.write(
        "serve.toml",
        &format!(
            "schema_version = \"agent-session.serve-config.v1\"\n[path]\nappend = [{}]\n",
            entries.join(", ")
        ),
    );

    let error = failure(&fixture, &fixture.check(&config, &[]));

    assert_eq!(error["code"], "serve-config-invalid-value");
    assert_eq!(error["details"]["key"], "path.append");
}

#[test]
fn inline_secrets_are_refused_without_echoing_the_value() {
    let fixture = Fixture::new();
    for (body, key) in [
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\napi_key = \"sk-inline-value\"\n",
            "retitle.api_key",
        ),
        ("token = \"sk-inline-value\"\n", "token"),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.extra_body]\nclient_secret = \"sk-inline-value\"\n",
            "retitle.extra_body.client_secret",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.extra_body]\napi_key_env = \"sk-inline-value\"\n",
            "retitle.extra_body.api_key_env",
        ),
        (
            "[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.extra_body]\nsecret_key = \"sk-inline-value\"\n",
            "retitle.extra_body.secret_key",
        ),
    ] {
        let config = fixture.write(
            "serve.toml",
            &format!("schema_version = \"agent-session.serve-config.v1\"\n{body}"),
        );
        let output = fixture.check(&config, &[]);
        let error = failure(&fixture, &output);
        assert_eq!(error["code"], "serve-config-inline-secret", "{body}");
        assert_eq!(error["details"]["key"], key, "{body}");
        for stream in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(stream).contains("sk-inline-value"));
        }
    }
}

#[test]
fn malformed_documents_fail_without_echoing_content() {
    let fixture = Fixture::new();
    let toml = fixture.write("serve.toml", "schema_version = \"sk-secret-shape\n");
    let json = fixture.write("serve.json", "{\"schema_version\": \"sk-secret-shape\"");

    for config in [toml, json] {
        let output = fixture.check(&config, &[]);
        let error = failure(&fixture, &output);
        assert_eq!(error["code"], "serve-config-parse-failed");
        for stream in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(stream).contains("sk-secret-shape"));
        }
    }
}

#[test]
fn unreadable_and_unsupported_files_are_path_free() {
    let fixture = Fixture::new();
    let missing = fixture.dir.path().join("absent.toml");
    let error = failure(&fixture, &fixture.check(&missing, &[]));
    assert_eq!(error["code"], "serve-config-unreadable");

    let yaml = fixture.write("serve.yaml", "schema_version: x\n");
    let error = failure(&fixture, &fixture.check(&yaml, &[]));
    assert_eq!(error["code"], "serve-config-unsupported-format");
}

#[test]
fn environment_values_override_the_file_and_are_reported() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);
    let retitle = r#"{"provider":"openai_compatible","base_url":"http://127.0.0.1:9/v1","model":"env-model"}"#;

    let output = fixture.check(
        &config,
        &[
            ("AGENT_SESSION_RETITLE_CONFIG", retitle),
            (
                "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
                r#"["/opt/env/broker"]"#,
            ),
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = envelope(&output);
    assert_eq!(value["data"]["retitle"]["source"], "environment");
    assert_eq!(
        value["data"]["codex_account_broker"]["source"],
        "environment"
    );
    let warnings = warnings(&value);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("AGENT_SESSION_RETITLE_CONFIG") && w.contains("retitle")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("AGENT_SESSION_CODEX_ACCOUNT_BROKER")
                && w.contains("codex_account_broker")),
        "{warnings:?}"
    );
}

#[test]
fn empty_environment_values_do_not_override_the_file() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);

    let output = fixture.check(
        &config,
        &[
            ("AGENT_SESSION_RETITLE_CONFIG", ""),
            ("AGENT_SESSION_CODEX_ACCOUNT_BROKER", "  "),
            ("AGENT_SESSION_LAUNCH_PROFILES", ""),
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &envelope(&output)["data"];
    assert_eq!(data["retitle"]["source"], "file");
    assert_eq!(data["codex_account_broker"]["source"], "file");
    assert_eq!(data["launch_profiles"]["source"], "file");
}

#[test]
fn environment_launch_profiles_merge_ahead_of_file_profiles() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);
    let env_profiles = r#"[
      {"id":"codex-alt","label":"Codex Alt","agent":"codex","agent_bin":"/opt/env/codex"},
      {"id":"dsh-alt","label":"DSH (env)","agent":"dsh","agent_bin":"/opt/env/dsh"}
    ]"#;

    let output = fixture.check(&config, &[("AGENT_SESSION_LAUNCH_PROFILES", env_profiles)]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = envelope(&output);
    assert_eq!(value["data"]["launch_profiles"]["source"], "merged");
    assert_eq!(
        value["data"]["launch_profiles"]["ids"],
        serde_json::json!(["codex-alt", "dsh-alt", "dsh-workbench"])
    );
    let warnings = warnings(&value);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("launch_profiles[0]") && w.contains("dsh-alt")),
        "{warnings:?}"
    );
}

#[test]
fn invalid_environment_launch_profiles_are_named_by_variable() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);

    let error = failure(
        &fixture,
        &fixture.check(&config, &[("AGENT_SESSION_LAUNCH_PROFILES", "{not json")]),
    );

    assert_eq!(error["code"], "serve-config-invalid-value");
    assert_eq!(error["details"]["key"], "AGENT_SESSION_LAUNCH_PROFILES");
    assert_eq!(error["details"]["source"], "environment");
}

#[test]
fn merged_launch_profiles_stay_within_the_profile_bound() {
    let fixture = Fixture::new();
    let profile = |prefix: &str, index: usize| {
        format!(
            r#"{{"id":"{prefix}{index}","label":"P {index}","agent":"codex","agent_bin":"/opt/p"}}"#
        )
    };
    let env_profiles = format!(
        "[{}]",
        (0..10)
            .map(|index| profile("env-", index))
            .collect::<Vec<_>>()
            .join(",")
    );
    let file_profiles = (0..10)
        .map(|index| profile("file-", index))
        .collect::<Vec<_>>()
        .join(",");
    let config = fixture.write(
        "serve.json",
        &format!(
            r#"{{"schema_version":"agent-session.serve-config.v1","launch_profiles":[{file_profiles}]}}"#
        ),
    );

    let error = failure(
        &fixture,
        &fixture.check(&config, &[("AGENT_SESSION_LAUNCH_PROFILES", &env_profiles)]),
    );

    assert_eq!(error["code"], "serve-config-invalid-value");
    assert_eq!(error["details"]["key"], "launch_profiles");
}

#[test]
fn text_check_reports_errors_on_stderr_with_the_key() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\n[path]\nappend = [\"bin\"]\n",
    );

    let output = fixture
        .command(&["--config", config.to_str().unwrap(), "--check"], &[])
        .output()
        .expect("run serve --check");

    assert_eq!(output.status.code(), Some(EXIT_USAGE), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("path.append[0]"), "{stderr}");
    assert!(
        !stderr.contains(&*fixture.dir.path().to_string_lossy()),
        "{stderr}"
    );
}

#[test]
fn check_requires_a_config_file() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["--check"], &[])
        .output()
        .expect("run serve --check");
    assert_eq!(output.status.code(), Some(EXIT_USAGE), "{output:?}");
}

#[test]
fn check_exits_without_taking_the_serve_lock_or_reading_a_token() {
    let fixture = Fixture::new();
    let config = fixture.write("serve.toml", VALID_TOML);

    // `--token-stdin` with a closed stdin would fail token resolution; a check
    // must finish before the daemon reads credentials or owns the state root.
    let output = fixture
        .command(
            &[
                "--config",
                config.to_str().unwrap(),
                "--check",
                "--token-stdin",
            ],
            &[],
        )
        .output()
        .expect("run serve --check");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!fixture.state_dir().join("serve.lock").exists());
}

#[test]
fn serve_refuses_to_start_on_an_invalid_config() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\nunknown = true\n",
    );

    let output = fixture
        .command(
            &[
                "--config",
                config.to_str().unwrap(),
                "--bind",
                "127.0.0.1:0",
            ],
            &[],
        )
        .output()
        .expect("run serve");

    assert_eq!(output.status.code(), Some(EXIT_USAGE), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown"), "{stderr}");
    assert!(!fixture.state_dir().join("serve.lock").exists());
}

#[test]
fn path_append_reports_the_configured_entry_count() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\n[path]\nappend = [\"/usr/bin\", \"/opt/example/bin\"]\n",
    );

    // `/usr/bin` is already inherited and is not appended twice, but the
    // summary reports the configured `path.append` count.
    let output = fixture.check(&config, &[("PATH", "/usr/bin:/bin")]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(envelope(&output)["data"]["path"]["append"], 2);
}

#[test]
fn provider_parameters_that_mention_tokens_are_not_secrets() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\n[retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n[retitle.extra_body]\nstop_token_ids = [1, 2]\n",
    );

    let output = fixture.check(&config, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(envelope(&output)["data"]["retitle"]["source"], "file");
}
