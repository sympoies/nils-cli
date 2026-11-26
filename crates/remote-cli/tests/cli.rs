use std::fs::{self, File, Permissions};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    remote_home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let remote_home = root.path().join("remote home");
        fs::create_dir(&remote_home).unwrap();
        for (name, body) in [
            ("ssh", include_str!("fixtures/ssh")),
            ("login-shell", include_str!("fixtures/login-shell")),
            ("sudo", include_str!("fixtures/sudo")),
        ] {
            let executable = root.path().join(name);
            fs::write(&executable, body).unwrap();
            fs::set_permissions(executable, Permissions::from_mode(0o700)).unwrap();
        }
        Self { root, remote_home }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_remote-cli"));
        command
            .env(
                "PATH",
                std::env::join_paths([self.root.path(), Path::new("/usr/bin"), Path::new("/bin")])
                    .unwrap(),
            )
            .env("HOME", self.root.path().join("local home"))
            .env("FIXTURE_REMOTE_HOME", &self.remote_home)
            .env("FIXTURE_REMOTE_SHELL", self.root.path().join("login-shell"));
        command
    }

    fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.path().join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

fn canonical_path_from_output(output: &str) -> PathBuf {
    Path::new(output.strip_suffix('\n').unwrap())
        .canonicalize()
        .unwrap()
}

fn result(output: &Output, code: i32, schema: &str) -> Value {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema_version"], schema);
    assert_eq!(result["ok"], code == 0);
    result
}

#[test]
fn exec_preserves_argv_and_expands_only_the_remote_home_prefix() {
    let fixture = Fixture::new();
    let suffix = "work '$HOME; \\";
    let workdir = fixture.remote_home.join(suffix);
    fs::create_dir(&workdir).unwrap();
    let args = [
        "space here",
        "one'and\"two",
        "$HOME",
        "",
        "line\nnext",
        "\\tail",
        "--format=json",
    ];
    for prefix in ["~/", "$HOME/"] {
        let cwd = format!("{prefix}{suffix}");
        let output = fixture
            .command()
            .args([
                "exec",
                "fixture.invalid",
                "--cwd",
                &cwd,
                "--format",
                "json",
                "--",
                "printf",
                "%s\\000",
                "$HOME",
            ])
            .args(args)
            .output()
            .unwrap();
        let payload = result(&output, 0, "cli.remote-cli.exec.v1");
        let data = &payload["data"];
        assert_eq!(data["host"], "fixture.invalid");
        assert_eq!(data["requested_cwd"], cwd);
        assert_eq!(
            &data["argv"].as_array().unwrap()[3..],
            serde_json::json!(args).as_array().unwrap()
        );
        assert_eq!(data["stdout"], format!("$HOME\0{}\0", args.join("\0")));
        assert_eq!(data["stderr"], "");
        assert_eq!(data["ssh_exit_code"], 0);
        assert_eq!(data["remote_outcome"], "succeeded");
        assert_eq!(data["connect_timeout_ms"], 6_000);
        assert_eq!(data["timeout_ms"], 40_000);
        assert!(data["elapsed_ms"].is_u64());
        assert!(
            data["started_at"]
                .as_str()
                .unwrap()
                .parse::<jiff::Timestamp>()
                .is_ok()
        );

        let output = fixture
            .command()
            .args(["exec", "fixture.invalid", "--cwd", &cwd, "--", "/bin/pwd"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let canonical = workdir.canonicalize().unwrap();
        assert_eq!(
            canonical_path_from_output(std::str::from_utf8(&output.stdout).unwrap()),
            canonical
        );
    }
}

#[test]
fn default_exec_leaves_caller_stdin_unread_and_explicit_file_is_forwarded() {
    let fixture = Fixture::new();
    let input = fixture.file("stdin.txt", b"caller stdin\n");
    let mut caller_stdin = File::open(&input).unwrap();
    let output = fixture
        .command()
        .args(["exec", "fixture.invalid", "--", "cat"])
        .stdin(caller_stdin.try_clone().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let mut remaining = Vec::new();
    caller_stdin.read_to_end(&mut remaining).unwrap();
    assert_eq!(remaining, b"caller stdin\n");

    let output = fixture
        .command()
        .env("EXPECT_STDIN_NULL", "no")
        .args(["exec", "fixture.invalid", "--stdin-file"])
        .arg(input)
        .args(["--", "cat"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"caller stdin\n");
}

#[test]
fn shell_reads_the_local_script_via_stdin_in_the_selected_login_environment() {
    let fixture = Fixture::new();
    let script = fixture.file(
        "file with spaces.sh",
        b"printf '%s\\n' \"$REMOTE_PROFILE\"\npwd\n",
    );
    let output = fixture
        .command()
        .env("EXPECT_STDIN_NULL", "no")
        .env("FIXTURE_REMOTE_SHELL", "/invalid-login-shell")
        .args(["shell", "fixture.invalid", "--script-file"])
        .arg(&script)
        .arg("--shell")
        .arg(fixture.root.path().join("login-shell"))
        .args(["--format", "json"])
        .output()
        .unwrap();
    let payload = result(&output, 0, "cli.remote-cli.shell.v1");
    let data = &payload["data"];
    assert_eq!(data["script_file"], script.to_str().unwrap());
    assert_eq!(data["argv"], Value::Null);
    assert_eq!(data["requested_cwd"], Value::Null);
    let (profile, pwd) = data["stdout"].as_str().unwrap().split_once('\n').unwrap();
    assert_eq!(profile, "loaded");
    assert_eq!(
        canonical_path_from_output(pwd),
        fixture.remote_home.canonicalize().unwrap()
    );
}

#[test]
fn sudo_refuses_a_password_prompt_as_a_remote_failure() {
    let output = Fixture::new()
        .command()
        .args([
            "exec",
            "fixture.invalid",
            "--sudo",
            "--sudo-user",
            "fixture-user",
            "--format",
            "json",
            "--",
            "echo",
            "must not run",
        ])
        .output()
        .unwrap();
    let payload = result(&output, 1, "cli.remote-cli.exec.v1");
    assert_eq!(payload["error"]["code"], "remote-exit");
    assert_eq!(payload["data"]["remote_outcome"], "failed");
    assert_eq!(payload["data"]["sudo_user"], "fixture-user");
    assert_eq!(payload["data"]["stdout"], "");
    assert_eq!(payload["data"]["stderr"], "sudo: a password is required\n");
}

#[test]
fn transport_error_and_remote_exit_255_preserve_the_same_ssh_code() {
    let fixture = Fixture::new();
    let disconnected = fixture
        .command()
        .env("FAKE_SSH_MODE", "disconnect")
        .args(["exec", "fixture.invalid", "--format", "json", "--", "true"])
        .output()
        .unwrap();
    let payload = result(&disconnected, 255, "cli.remote-cli.exec.v1");
    assert_eq!(payload["error"]["code"], "ssh-transport");
    assert_eq!(payload["data"]["remote_outcome"], "unknown");
    assert_eq!(payload["data"]["timed_out"], false);
    assert_eq!(payload["data"]["stdout"], "partial transport output");
    assert_eq!(payload["data"]["stderr"], "connection closed");
    assert_eq!(payload["data"]["ssh_exit_code"], 255);

    let remote = fixture
        .command()
        .args([
            "exec",
            "fixture.invalid",
            "--format",
            "json",
            "--",
            "/bin/sh",
            "-c",
            "printf 'remote stderr' >&2; exit 255",
        ])
        .output()
        .unwrap();
    let payload = result(&remote, 255, "cli.remote-cli.exec.v1");
    assert_eq!(payload["error"]["code"], "remote-exit");
    assert_eq!(payload["data"]["remote_outcome"], "failed");
    assert_eq!(payload["data"]["ssh_exit_code"], 255);
    assert_eq!(payload["data"]["stderr"], "remote stderr");
}

#[test]
fn deadline_preserves_partial_output_and_differs_from_remote_exit_124() {
    let fixture = Fixture::new();
    let timed_out = fixture
        .command()
        .env("EXPECT_CONNECT_TIMEOUT", "2")
        .args([
            "exec",
            "fixture.invalid",
            "--format",
            "json",
            "--timeout",
            "1",
            "--connect-timeout",
            "2",
            "--",
            "/bin/sh",
            "-c",
            "printf 'partial stdout'; printf 'partial stderr' >&2; sleep 20",
        ])
        .output()
        .unwrap();
    let payload = result(&timed_out, 124, "cli.remote-cli.exec.v1");
    assert_eq!(payload["error"]["code"], "deadline-exceeded");
    assert_eq!(payload["data"]["remote_outcome"], "unknown");
    assert_eq!(payload["data"]["timed_out"], true);
    assert_eq!(payload["data"]["ssh_exit_code"], Value::Null);
    assert_eq!(payload["data"]["signal"], libc::SIGKILL);
    assert_eq!(payload["data"]["stdout"], "partial stdout");
    assert_eq!(payload["data"]["stderr"], "partial stderr");
    assert_eq!(payload["data"]["timeout_ms"], 1000);
    assert_eq!(payload["data"]["connect_timeout_ms"], 2_000);
    assert!(payload["data"]["elapsed_ms"].as_u64().unwrap() < 10_000);

    let remote = fixture
        .command()
        .args([
            "exec",
            "fixture.invalid",
            "--format",
            "json",
            "--timeout",
            "0",
            "--",
            "/bin/sh",
            "-c",
            "exit 124",
        ])
        .output()
        .unwrap();
    let payload = result(&remote, 124, "cli.remote-cli.exec.v1");
    assert_eq!(payload["error"]["code"], "remote-exit");
    assert_eq!(payload["data"]["remote_outcome"], "failed");
    assert_eq!(payload["data"]["timed_out"], false);
    assert_eq!(payload["data"]["ssh_exit_code"], 124);
    assert_eq!(payload["data"]["timeout_ms"], 0);
}

#[test]
fn help_and_exit_matrix_cover_usage_input_and_missing_transport() {
    let fixture = Fixture::new();
    let help = fixture.command().arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for content in ["--version", "--cwd", "--sudo", "--script-file", "$HOME"] {
        assert!(help.contains(content), "{help}");
    }
    let version = fixture.command().arg("-V").output().unwrap();
    assert!(version.status.success());
    assert!(version.stdout.starts_with(b"remote-cli "));

    let usage = fixture
        .command()
        .args(["exec", "fixture.invalid", "--format", "json", "true"])
        .output()
        .unwrap();
    assert_eq!(
        result(&usage, 64, "cli.remote-cli.error.v1")["error"]["code"],
        "parse-error"
    );
    let input = fixture
        .command()
        .args([
            "shell",
            "fixture.invalid",
            "--format",
            "json",
            "--script-file",
        ])
        .arg(fixture.root.path().join("missing"))
        .output()
        .unwrap();
    assert_eq!(
        result(&input, 65, "cli.remote-cli.shell.v1")["error"]["code"],
        "input-file"
    );
    let unavailable = fixture
        .command()
        .env("PATH", &fixture.remote_home)
        .args(["exec", "fixture.invalid", "--format", "json", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(
        result(&unavailable, 69, "cli.remote-cli.exec.v1")["error"]["code"],
        "ssh-unavailable"
    );
}
