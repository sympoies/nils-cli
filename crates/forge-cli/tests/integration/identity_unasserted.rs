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
            "#!/bin/sh\ncase \"$1:$2\" in\n auth:token) printf '%s' \"$FIXTURE_CREDENTIAL\"; exit 0;;\n api:user) printf '{\"login\":\"account-a\"}'; exit 0;;\nesac\nprintf '{\"number\":1,\"url\":\"https://github.com/sandbox/widget/issues/1\",\"state\":\"OPEN\",\"title\":\"Example\",\"body\":\"\",\"labels\":[],\"assignees\":[]}'\n",
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).unwrap();
        Self { home }
    }
    fn run(&self, principal: Option<&str>) -> Output {
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
            .env("AGENT_SESSION_ID", "SESSION_CANARY_91")
            .env_remove("FORGE_IDENTITY_PRINCIPAL");
        if let Some(principal) = principal {
            command.env("FORGE_IDENTITY_PRINCIPAL", principal);
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
