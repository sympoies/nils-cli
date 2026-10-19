//! Startup failures must cross the same create boundary as successful launches.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use pretty_assertions::assert_eq;
use serde_json::Value;

struct Fixture {
    dir: tempfile::TempDir,
    tmux: PathBuf,
    provider: PathBuf,
}

impl Fixture {
    fn new(body: &str) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        for name in ["repo", "home"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        let tmux = dir.path().join("tmux");
        executable(
            &tmux,
            &format!(
                "#!/bin/sh\nexec tmux -S {} \"$@\"\n",
                shell_words::quote(&dir.path().join("tmux.sock").to_string_lossy())
            ),
        );
        let provider = dir.path().join("provider");
        executable(&provider, &format!("#!/bin/sh\n{body}\n"));
        Self {
            dir,
            tmux,
            provider,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        // No inherited session identity, account broker, or provider credentials.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("AGENT_SESSION_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", self.dir.path().join("home"))
            .env("CODEX_HOME", self.dir.path().join("home"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .args([
                "--state-dir",
                self.dir.path().join("state").to_str().unwrap(),
            ]);
        command
    }

    fn start_command(&self) -> Command {
        let mut command = self.command();
        command
            .args([
                "start",
                "--agent",
                "codex",
                "--id",
                "startup-fixture",
                "--cwd",
            ])
            .arg(self.dir.path().join("repo"))
            .arg("--tmux-bin")
            .arg(&self.tmux)
            .arg("--agent-bin")
            .arg(&self.provider)
            .args(["--format", "json"])
            .stdin(Stdio::null());
        command
    }

    fn start(&self) -> Output {
        self.start_command().output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new(&self.tmux).arg("kill-server").output();
    }
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn startup_exit_returns_typed_failure_with_exit_code_and_redacted_stderr() {
    let fixture = Fixture::new(
        "printf '%s\\n' 'provider configuration unavailable token=fixture-secret' >&2\nexit 23",
    );
    let output = fixture.start();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["error"]["code"], "start-failed");
    let details = &body["error"]["details"];
    assert_eq!(details["reason"], "runtime-exited-during-startup");
    assert_eq!(details["provider_exit_code"], 23);
    assert_eq!(details["retryable"], true, "{details}");
    assert_eq!(details["startup"]["state"], "failed");
    assert!(
        details["stderr_tail"]
            .as_str()
            .unwrap()
            .contains("provider configuration unavailable")
    );
    assert!(!body.to_string().contains("fixture-secret"));
    assert!(
        !body
            .to_string()
            .contains(fixture.dir.path().to_str().unwrap())
    );
}

#[test]
fn exited_provider_is_detected_while_descendant_holds_stderr_open() {
    let fixture = Fixture::new("sleep 2 &\nprintf '%s\\n' 'startup stderr holder' >&2\nexit 23");
    let output = fixture
        .start_command()
        .env("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "250")
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    assert_eq!(body["error"]["details"]["provider_exit_code"], 23);
    assert_eq!(body["error"]["details"]["cleanup"]["state"], "completed");
    assert_eq!(body["error"]["details"]["retryable"], true);
}

#[test]
fn delayed_startup_exit_without_stderr_is_still_retryable() {
    let fixture = Fixture::new("sleep 3\nexit 1");
    let output = fixture
        .start_command()
        .env("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "30000")
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    assert_eq!(body["error"]["details"]["provider_exit_code"], 1);
    assert_eq!(body["error"]["details"]["retryable"], true);
}

#[test]
fn trust_prompt_fails_without_leaving_a_live_orphan() {
    let fixture = Fixture::new(
        "printf '%s\\n' 'Trust this folder?' '1. Yes, proceed' '2. No, exit'\nsleep 60",
    );
    let output = fixture.start();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    let details = &body["error"]["details"];
    assert_eq!(details["reason"], "startup-trust-required");
    assert_eq!(details["next_action"], "configure_provider_trust");
    assert_eq!(details["startup"]["state"], "failed");
    assert_eq!(details["cleanup"]["state"], "completed");
    assert_eq!(details["retryable"], false);
    let live = Command::new(&fixture.tmux)
        .args(["has-session", "-t", "hs-codex-startup-fixture"])
        .output()
        .unwrap();
    assert!(
        !live.status.success(),
        "startup prompt must not leave a live orphan"
    );
}

struct Daemon(std::process::Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn http_create_reports_app_server_exit_and_retains_reconcilable_session() {
    let fixture = Fixture::new(
        r#"
case "$*" in
  --version) echo 'codex-cli 0.162.0'; exit 0 ;;
  'app-server --help') echo '  --listen <URL> unix://'; exit 0 ;;
esac
printf '%s\n' 'provider startup configuration unavailable' >&2
exit 23
"#,
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut daemon = Daemon(
        fixture
            .command()
            .args(["serve", "--bind", &address.to_string()])
            .env("AGENT_SESSION_TMUX_BIN", &fixture.tmux)
            .env("AGENT_SESSION_CODEX_BIN", &fixture.provider)
            .env("AGENT_SESSION_CODEX_RUNTIME", "app-server")
            .env("AGENT_SESSION_TOKEN", "fixture-token")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::net::TcpStream::connect(address).is_err() {
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited");
        assert!(
            std::time::Instant::now() < deadline,
            "daemon did not listen"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let url = format!("http://{address}/sessions");
    let response = client.post(&url).bearer_auth("fixture-token")
        .json(&serde_json::json!({"agent":"codex", "id":"startup-fixture", "cwd":fixture.dir.path().join("repo")}))
        .send().unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    let details = &body["error"]["details"];
    assert_eq!(details["session_id"], "startup-fixture");
    assert_eq!(details["provider_exit_code"], 23);
    assert_eq!(
        details["startup"]["failure_code"],
        "app-server-start-failed"
    );
    assert_eq!(details["retryable"], true, "{details}");
    assert!(
        details["stderr_tail"]
            .as_str()
            .unwrap()
            .contains("provider startup configuration unavailable")
    );
    let listed: Value = client
        .get(&url)
        .bearer_auth("fixture-token")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(listed["data"]["sessions"][0]["startup"]["state"], "failed");
}

#[test]
fn zero_exit_before_readiness_is_a_start_failure() {
    let fixture = Fixture::new("exit 0");
    let output = fixture.start();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    assert_eq!(body["error"]["details"]["provider_exit_code"], 0);
}

#[test]
fn normal_exit_after_the_composer_was_ready_remains_successful() {
    let fixture = Fixture::new("printf '› \\n\\n  status\\n'\nsleep 1\nexit 0");
    let output = fixture.start();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["data"]["startup"]["state"], "ready");

    assert_eq!(output.status.code(), Some(0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while Command::new(&fixture.tmux)
        .args(["has-session", "-t", "hs-codex-startup-fixture"])
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "provider did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let listed = fixture
        .command()
        .env("AGENT_SESSION_TMUX_BIN", &fixture.tmux)
        .args(["list", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["data"][0]["status"], "stopped");
    assert_eq!(listed["data"][0]["startup"]["state"], "ready");
}

#[test]
fn stderr_tail_is_bounded_and_keeps_the_final_startup_reason() {
    let fixture = Fixture::new(
        r#"
printf '%s\n' 'PREFIX_MUST_BE_DROPPED' >&2
i=0
while [ "$i" -lt 4000 ]; do
  printf '%s\n' 'diagnostic padding line' >&2
  i=$((i + 1))
done
printf '%s\n' 'FINAL_STARTUP_REASON path=/private/example/config token=fixture-secret' >&2
exit 7
"#,
    );
    let started = std::time::Instant::now();
    let output = fixture.start();
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    let details = &body["error"]["details"];
    assert_eq!(details["provider_exit_code"], 7);
    let tail = details["stderr_tail"].as_str().unwrap();
    assert!(tail.len() <= 4096);
    assert!(tail.contains("FINAL_STARTUP_REASON"), "{tail}");
    assert!(!tail.contains("PREFIX_MUST_BE_DROPPED"));
    assert!(!tail.contains("fixture-secret"));
    assert!(!tail.contains("/private/example"));
}

#[test]
fn sign_in_screen_has_a_typed_configuration_reason() {
    let fixture =
        Fixture::new("printf '%s\n' 'Sign in with ChatGPT' 'Use your own API key'\nsleep 60");
    let body: Value = serde_json::from_slice(&fixture.start().stdout).unwrap();
    assert_eq!(body["error"]["code"], "start-failed", "{body}");
    assert_eq!(
        body["error"]["details"]["reason"],
        "startup-prompt-required"
    );
    assert_eq!(
        body["error"]["details"]["next_action"],
        "configure_provider"
    );
}
