use nils_test_support::bin::resolve;
use pretty_assertions::assert_eq;
use std::process::Command;

#[test]
fn identity_effects_distinguish_metadata_reads_from_doctor_network_and_audit() {
    for (command, effect, provider) in [
        ("explain", "read_only", "none"),
        ("doctor", "mutation", "network_read"),
    ] {
        let out = Command::new(resolve("forge-cli"))
            .args([
                "operation-effect",
                "--format",
                "json",
                "--",
                "identity",
                command,
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["data"]["effect"], effect);
        assert_eq!(v["data"]["provider_effect"], provider);
    }
}

#[test]
fn identity_explain_without_policy_is_read_only_and_disabled() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(resolve("forge-cli"))
        .args([
            "--format",
            "json",
            "--repo",
            "example/widget",
            "identity",
            "explain",
        ])
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["data"]["enforced"], false);
    assert!(!home.path().join("state").exists());
}

#[cfg(unix)]
mod policy_tests {
    use super::{Command, resolve};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    const POLICY: &str = include_str!("../../../nils-common/tests/fixtures/identity/policy.toml");
    const CANARY: &str = "FIXTURE_CANARY_NEVER_OUTPUT_83";
    struct Fixture {
        home: tempfile::TempDir,
        gh: std::path::PathBuf,
    }
    impl Fixture {
        fn new(policy: &str) -> Self {
            let home = tempfile::tempdir().unwrap();
            fs::create_dir(home.path().join("forge-cli")).unwrap();
            fs::write(home.path().join("forge-cli/identity.toml"), policy).unwrap();
            let gh = home.path().join("gh");
            fs::write(&gh, r#"#!/bin/sh
case "$1:$2" in
 auth:token) test "$5" = --user && test "$6" = account-b || exit 9; printf '%s' "$FIXTURE_SECRET_B"; exit 0;;
 api:user) printf '{"login":"%s"}' "${FIXTURE_ACTOR:-account-a}"; exit 0;;
 api:apps/fixture-app) printf '{"id":42}'; exit 0;;
 api:graphql) printf '{"data":{"viewer":{"login":"fixture-app[bot]"}}}'; exit 0;;
 api:installation/repositories*) printf '%s' "${FIXTURE_COVERAGE:-[]}"; exit 0;;
esac
if test -n "$GITHUB_TOKEN" || test -n "$GH_DEBUG"; then echo inherited-secret >&2; exit 9; fi
printf 'actor call\n' >> "$FIXTURE_CALL_LOG"
printf '{"number":1,"url":"https://github.com/sandbox/widget/issues/1","state":"OPEN","title":"Example","body":"%s","labels":[],"assignees":[]}' "$GH_TOKEN"
printf '%s' "$GH_TOKEN" >&2
"#).unwrap();
            fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).unwrap();
            Self { home, gh }
        }
        fn command(&self) -> Command {
            let mut c = Command::new(resolve("forge-cli"));
            c.args(["--format", "json", "--repo", "sandbox/widget"])
                .env("XDG_CONFIG_HOME", self.home.path())
                .env("XDG_STATE_HOME", self.home.path().join("state"))
                .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
                .env("FORGE_CLI_GH_BIN", &self.gh)
                .env("FORGE_CLI_RATE_LIMIT_GATE", "off")
                .env("FIXTURE_ACCOUNT_A_CREDENTIAL", CANARY)
                .env("FIXTURE_SECRET_B", CANARY)
                .env("FIXTURE_CALL_LOG", self.home.path().join("calls"));
            c
        }
        fn audit(&self) -> String {
            fs::read_to_string(
                self.home
                    .path()
                    .join("state/forge-cli/identity-audit.jsonl"),
            )
            .unwrap()
        }
    }
    #[test]
    fn identity_explain_is_metadata_only_and_two_principals_select_different_profiles() {
        let f = Fixture::new(POLICY);
        for (principal, profile) in [("contributor", "account-a"), ("coordinator", "account-b")] {
            let out = f
                .command()
                .env("FORGE_IDENTITY_PRINCIPAL", principal)
                .args(["identity", "explain"])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["data"]["selection"]["profile_id"], profile);
            assert!(!String::from_utf8_lossy(&out.stdout).contains(CANARY));
        }
        assert!(!f.home.path().join("calls").exists());
        assert!(!f.home.path().join("state").exists());
    }
    #[test]
    fn identity_missing_credential_and_actor_mismatch_refuse_without_fallback() {
        for (missing, expected) in [
            (true, "identity_credential_missing"),
            (false, "identity_actor_mismatch"),
        ] {
            let f = Fixture::new(POLICY);
            let mut c = f.command();
            c.env("GH_TOKEN", CANARY)
                .env("GITHUB_TOKEN", CANARY)
                .env("FIXTURE_ACTOR", "wrong-actor");
            if missing {
                c.env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL");
            }
            let out = c.args(["issue", "view", "1"]).output().unwrap();
            assert_eq!(out.status.code(), Some(65));
            assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
            assert!(!String::from_utf8_lossy(&out.stdout).contains(CANARY));
            assert!(!String::from_utf8_lossy(&out.stderr).contains(CANARY));
            assert!(!f.home.path().join("calls").exists());
            assert!(f.audit().contains(expected));
            assert!(!f.audit().contains(CANARY));
        }
    }
    #[test]
    fn identity_api_stdout_stderr_and_audit_never_contain_token_canary() {
        let f = Fixture::new(POLICY);
        let out = f
            .command()
            .env("GH_DEBUG", "api")
            .env("GITHUB_TOKEN", "WRONG_AMBIENT_CREDENTIAL")
            .args(["issue", "view", "1"])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["data"]["body"], "[REDACTED]");
        for s in [
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            f.audit(),
        ] {
            assert!(!s.contains(CANARY));
        }
    }
    #[test]
    fn identity_named_credential_uses_configured_user_and_verifies_actor() {
        let f = Fixture::new(POLICY);
        let out = f
            .command()
            .env("FORGE_IDENTITY_PRINCIPAL", "coordinator")
            .env("FIXTURE_ACTOR", "account-b")
            .args(["issue", "view", "1"])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(f.audit().contains("account-b"));
    }
    #[test]
    fn identity_app_id_actor_and_repository_coverage_are_all_required() {
        let policy = POLICY.replacen(
            "expected_login = \"account-a\"",
            "expected_app_id=42\napp_slug='fixture-app'",
            1,
        );
        for covered in [false, true] {
            let f = Fixture::new(&policy);
            let mut c = f.command();
            if covered {
                c.env(
                    "FIXTURE_COVERAGE",
                    r#"[{"repositories":[{"full_name":"sandbox/widget"}]}]"#,
                );
            }
            let out = c.args(["issue", "view", "1"]).output().unwrap();
            assert_eq!(
                out.status.code(),
                Some(if covered { 0 } else { 65 }),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            if !covered {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains("identity_app_repository_missing")
                );
                assert!(!f.home.path().join("calls").exists());
            }
        }
    }
    #[test]
    fn identity_doctor_missing_key_is_a_refusal_and_is_audited() {
        let f = Fixture::new(POLICY);
        let stubs = nils_test_support::StubBinDir::new();
        stubs.write_exe("gpg", "#!/bin/sh\nexit 2\n");
        let out = f
            .command()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    stubs.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .args(["identity", "doctor"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        assert!(String::from_utf8_lossy(&out.stdout).contains("identity_signing_key_missing"));
        assert!(f.audit().contains("identity_signing_key_missing"));
        assert!(!f.audit().contains(CANARY));
    }
    #[test]
    fn identity_unknown_principal_ambiguous_rules_and_strict_schema_refuse() {
        for (policy, principal, expected) in [
            (POLICY.to_string(), "unknown", "identity_principal_unknown"),
            (
                format!(
                    "{POLICY}\n[[rules]]\nid='conflict'\nprincipal='contributor'\nrepo='github.com/sandbox/widget'\nprofile='account-b'"
                ),
                "contributor",
                "identity_rule_ambiguous",
            ),
            (
                format!("token='{CANARY}'\n{POLICY}"),
                "contributor",
                "identity_policy_invalid",
            ),
        ] {
            let f = Fixture::new(&policy);
            let out = f
                .command()
                .env("FORGE_IDENTITY_PRINCIPAL", principal)
                .args(["issue", "view", "1"])
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(65));
            assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
            assert!(!String::from_utf8_lossy(&out.stdout).contains(CANARY));
            assert!(!f.home.path().join("calls").exists());
        }
    }
}
