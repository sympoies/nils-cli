//! A non-interactive run that asserts no principal under an `asserted-only` policy.
use nils_test_support::bin::resolve;
use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

const POLICY: &str = include_str!("../../../nils-common/tests/fixtures/identity/policy.toml");
const CANARY: &str = "FIXTURE_CANARY_NEVER_OUTPUT_91";

struct Fixture {
    home: tempfile::TempDir,
}
impl Fixture {
    fn new(key: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("forge-cli")).unwrap();
        fs::write(
            home.path().join("forge-cli/identity.toml"),
            format!("activation='asserted-only'\n{key}{POLICY}"),
        )
        .unwrap();
        let gh = home.path().join("gh");
        fs::write(
            &gh,
            "#!/bin/sh\nprintf 'call\\n' >> \"$FIXTURE_CALL_LOG\"\ncase \"$1:$2\" in\n auth:token) printf '%s' \"$FIXTURE_CREDENTIAL\"; exit 0;;\n api:user) printf '{\"login\":\"account-a\"}'; exit 0;;\nesac\nprintf '{\"number\":1,\"url\":\"https://github.com/sandbox/widget/issues/1\",\"state\":\"OPEN\",\"title\":\"Example\",\"body\":\"\",\"labels\":[],\"assignees\":[]}'\n",
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).unwrap();
        Self { home }
    }
    fn base(&self) -> Command {
        let mut command = Command::new(resolve("forge-cli"));
        command
            .args([
                "--format",
                "json",
                "--repo",
                "sandbox/widget",
                "issue",
                "view",
                "1",
            ])
            .env("HOME", self.home.path())
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_CONFIG_HOME", self.home.path())
            .env("XDG_STATE_HOME", self.home.path().join("state"))
            .env("FORGE_CLI_GH_BIN", self.home.path().join("gh"))
            .env("FORGE_CLI_RATE_LIMIT_GATE", "off")
            .env("FIXTURE_CREDENTIAL", CANARY)
            .env("FIXTURE_CALL_LOG", self.home.path().join("calls"))
            .env_remove("FORGE_IDENTITY_PRINCIPAL");
        command
    }
    fn run(&self, principal: Option<&str>) -> Output {
        let mut command = self.base();
        command.env("AGENT_SESSION_ID", "SESSION_CANARY_91");
        if let Some(principal) = principal {
            command.env("FORGE_IDENTITY_PRINCIPAL", principal);
        }
        command.output().unwrap()
    }
    /// Run with no automation marker, giving the child a pseudo-terminal on the named streams.
    fn run_with_terminal(&self, terminal: &str) -> Output {
        self.run_with_terminal_and_marker(terminal, false)
    }
    fn run_with_terminal_and_marker(&self, terminal: &str, marker: bool) -> Output {
        let base = self.base();
        let mut command = Command::new("python3");
        command.args([
            "-c",
            "import subprocess,sys,pty\nm,s=pty.openpty()\nmode=sys.argv[1]\np=subprocess.run(sys.argv[2:],stdin=s if mode in('both','stdin') else subprocess.DEVNULL,stdout=s if mode in('both','stdout') else subprocess.PIPE,stderr=subprocess.PIPE,timeout=20)\nsys.stderr.buffer.write(p.stderr)\nsys.exit(p.returncode)\n",
            terminal,
        ]);
        command.arg(base.get_program()).args(base.get_args());
        for (key, value) in base.get_envs() {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        for name in ["INVOCATION_ID", "GITHUB_ACTIONS", "AGENT_SESSION_ID"] {
            command.env_remove(name);
        }
        if marker {
            command.env("AGENT_SESSION_ID", "SESSION_CANARY_91");
        }
        command.output().unwrap()
    }
    fn audit(&self) -> String {
        fs::read_to_string(
            self.home
                .path()
                .join("state/forge-cli/identity-audit.jsonl"),
        )
        .unwrap_or_default()
    }
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn warn_names_the_fix_once_keeps_the_run_and_audits_caller_only() {
    for key in ["", "unasserted_noninteractive='warn'\n"] {
        let f = Fixture::new(key);
        let out = f.run(None);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let err = stderr(&out);
        assert_eq!(err.matches("FORGE_IDENTITY_PRINCIPAL").count(), 1, "{err}");
        assert!(err.contains("unasserted_noninteractive"), "{err}");
        let audit = f.audit();
        assert_eq!(audit.lines().count(), 1, "{audit}");
        assert!(audit.contains("unasserted_noninteractive_warned"));
        for (index, secret) in [CANARY, "SESSION_CANARY_91", f.home.path().to_str().unwrap()]
            .into_iter()
            .enumerate()
        {
            assert!(
                !audit.contains(secret) && !err.contains(secret),
                "output leaked value #{index}"
            );
        }
    }
}

#[test]
fn deny_refuses_with_data_exit_stable_code_and_fix_text() {
    let f = Fixture::new("unasserted_noninteractive='deny'\n");
    let out = f.run(None);
    assert_eq!(out.status.code(), Some(65));
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(
        body.contains("identity_unasserted_noninteractive"),
        "{body}"
    );
    assert!(body.contains("FORGE_IDENTITY_PRINCIPAL"), "{body}");
    assert!(f.audit().contains("unasserted_noninteractive_denied"));
}

#[test]
fn allow_and_asserted_principal_are_silent_in_every_mode() {
    let f = Fixture::new("unasserted_noninteractive='allow'\n");
    let out = f.run(None);
    assert_eq!(out.status.code(), Some(0));
    assert!(!stderr(&out).contains("FORGE_IDENTITY_PRINCIPAL"));
    assert_eq!(f.audit(), "");
    for key in ["allow", "warn", "deny"] {
        let f = Fixture::new(&format!("unasserted_noninteractive='{key}'\n"));
        let out = f.run(Some("contributor"));
        assert!(!stderr(&out).contains("unasserted_noninteractive"), "{key}");
        assert!(!f.audit().contains("unasserted_noninteractive"), "{key}");
    }
}

#[test]
fn warn_and_deny_add_no_provider_calls() {
    let calls = |key: &str| {
        let f = Fixture::new(key);
        f.run(None);
        fs::read_to_string(f.home.path().join("calls"))
            .map(|log| log.lines().count())
            .unwrap_or(0)
    };
    let allow = calls("unasserted_noninteractive='allow'\n");
    assert!(allow > 0);
    assert_eq!(calls("unasserted_noninteractive='warn'\n"), allow);
    assert_eq!(calls("unasserted_noninteractive='deny'\n"), 0);
}

#[test]
fn terminals_on_both_streams_are_silent_and_one_terminal_still_warns() {
    let f = Fixture::new("");
    let out = f.run_with_terminal("both");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("FORGE_IDENTITY_PRINCIPAL"));
    assert_eq!(f.audit(), "");
    for terminal in ["stdin", "stdout", "none"] {
        let f = Fixture::new("");
        let out = f.run_with_terminal(terminal);
        assert_eq!(out.status.code(), Some(0), "{terminal}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("FORGE_IDENTITY_PRINCIPAL"),
            "{terminal}"
        );
    }
}

#[test]
fn terminals_on_both_streams_still_warn_under_an_automation_marker() {
    let f = Fixture::new("");
    let out = f.run_with_terminal_and_marker("both", true);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("FORGE_IDENTITY_PRINCIPAL"));
    assert!(f.audit().contains("unasserted_noninteractive_warned"));
}

#[test]
fn a_stderr_pipe_whose_reader_exited_never_fails_a_warned_run() {
    let f = Fixture::new("");
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut command = f.base();
    let out = command
        .env("AGENT_SESSION_ID", "SESSION_CANARY_91")
        .stderr(writer)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(f.audit().contains("unasserted_noninteractive_warned"));
}

#[test]
fn a_closed_stderr_or_unwritable_audit_never_fails_a_warned_run() {
    let f = Fixture::new("");
    let base = f.base();
    let out = Command::new("sh")
        .args(["-c", "exec \"$0\" \"$@\" 2>&-"])
        .arg(base.get_program())
        .args(base.get_args())
        .envs(base.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))))
        .env_remove("FORGE_IDENTITY_PRINCIPAL")
        .env("AGENT_SESSION_ID", "SESSION_CANARY_91")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));

    let f = Fixture::new("");
    fs::write(f.home.path().join("state"), b"not a directory").unwrap();
    let out = f.run(None);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("FORGE_IDENTITY_PRINCIPAL"));
}

#[test]
fn identity_diagnostics_are_not_gated_under_deny() {
    let f = Fixture::new("unasserted_noninteractive='deny'\n");
    for command in ["explain", "doctor"] {
        let base = f.base();
        let out = Command::new(base.get_program())
            .args([
                "--format",
                "json",
                "--repo",
                "sandbox/widget",
                "identity",
                command,
            ])
            .envs(base.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))))
            .env("AGENT_SESSION_ID", "SESSION_CANARY_91")
            .env_remove("FORGE_IDENTITY_PRINCIPAL")
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&out.stdout).contains("identity_unasserted_noninteractive"),
            "{command}"
        );
    }
    assert!(!f.audit().contains("unasserted_noninteractive"));
}
