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
            let mut c = self.bare_command();
            c.args(["--repo", "sandbox/widget"]);
            c
        }
        fn bare_command(&self) -> Command {
            let mut c = Command::new(resolve("forge-cli"));
            c.args(["--format", "json"])
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

    fn cross_repo_fixture(policy: &str) -> Fixture {
        let f = Fixture::new(policy);
        fs::write(&f.gh, r#"#!/bin/sh
case "$1:$2" in
 auth:token) printf '%s' "$FIXTURE_SECRET_B"; exit 0;;
 api:user) printf '{"login":"%s"}' "${FIXTURE_ACTOR:-account-a}"; exit 0;;
esac
if test "$GH_TOKEN" != "$FIXTURE_ACCOUNT_A_CREDENTIAL" || test -n "$GITHUB_TOKEN"; then exit 9; fi
printf 'actor call\n' >> "$FIXTURE_CALL_LOG"
case "$1:$2" in
 api:graphql) printf '{"data":{"user":{"contributionsCollection":{"totalCommitContributions":0,"commitContributionsByRepository":[]}},"repository":{"issueOrPullRequest":{"timelineItems":{"nodes":[]}}}}}';;
 *) printf '[]';;
esac
"#).unwrap();
        f
    }

    fn single_profile_policy() -> String {
        POLICY
            .replace(
                "profiles = [\"account-a\", \"account-b\"]",
                "profiles = [\"account-a\"]",
            )
            .replace(
                "org = \"github.com/sandbox\"\nprofile = \"account-b\"",
                "org = \"github.com/sandbox\"\nprofile = \"account-a\"",
            )
    }

    #[test]
    fn identity_cross_repo_reads_resolve_single_profile_without_checkout() {
        let f = cross_repo_fixture(&single_profile_policy());
        for args in [
            vec!["inbox", "list"],
            vec!["inbox", "status"],
            vec!["inbox", "next"],
            vec!["activity", "commits"],
            vec!["activity", "events"],
            vec!["activity", "summary"],
        ] {
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .env("GITHUB_TOKEN", "WRONG_AMBIENT_CREDENTIAL")
                .env("FORGE_CLI_INBOX_NO_CACHE", "1")
                .args(["--provider", "github"])
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        assert!(f.home.path().join("calls").exists());
        assert!(f.audit().contains("account-a"));
        assert!(!f.audit().contains(CANARY));
    }

    #[test]
    fn identity_cross_repo_hosts_are_authorized_before_credential_probes() {
        let f = cross_repo_fixture(&single_profile_policy());
        fs::write(
            &f.gh,
            r#"#!/bin/sh
printf 'probe call\n' >> "$FIXTURE_CALL_LOG"
printf '{"login":"unlisted-actor"}'
"#,
        )
        .unwrap();
        for args in [
            vec!["activity", "commits"],
            vec!["activity", "events"],
            vec!["activity", "summary"],
        ] {
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .args(["--provider", "github", "--host", "unlisted.example"])
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(65), "{args:?}");
            assert!(
                String::from_utf8_lossy(&out.stdout).contains("identity_repository_unknown"),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
            assert!(
                !f.home.path().join("calls").exists(),
                "unlisted host must not be probed"
            );
        }
    }

    #[test]
    fn identity_cross_repo_reads_accept_explicitly_authorized_custom_hosts() {
        let f = cross_repo_fixture(
            &single_profile_policy().replace("github.com", "authorized.example"),
        );
        fs::write(&f.gh, r#"#!/bin/sh
printf 'probe call\n' >> "$FIXTURE_CALL_LOG"
if test "$GH_ENTERPRISE_TOKEN" != "$FIXTURE_ACCOUNT_A_CREDENTIAL" || test -n "$GH_TOKEN"; then exit 9; fi
case "$1:$2" in
 api:user) printf '{"login":"account-a"}';;
 api:graphql) printf '{"data":{"user":{"contributionsCollection":{"totalCommitContributions":0,"commitContributionsByRepository":[]}}}}';;
 *) printf '[]';;
esac
"#).unwrap();
        for args in [
            vec!["activity", "commits"],
            vec!["activity", "events"],
            vec!["activity", "summary"],
        ] {
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .args(["--provider", "github", "--host", "authorized.example"])
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        assert!(f.home.path().join("calls").exists());
    }

    #[test]
    fn identity_repo_scoped_reads_use_normal_rules_including_inbox_threads() {
        let f = cross_repo_fixture(POLICY);
        for args in [
            vec!["inbox", "list"],
            vec!["inbox", "status"],
            vec!["inbox", "next"],
            vec!["activity", "commits"],
            vec!["activity", "events"],
            vec!["activity", "summary"],
            vec!["activity", "feed"],
            vec!["search", "issues", "example"],
            vec!["search", "prs", "example"],
            vec!["search", "refs-to", "1"],
        ] {
            let out = f
                .command()
                .current_dir(f.home.path())
                .env("FORGE_CLI_INBOX_NO_CACHE", "1")
                .args(["--provider", "github"])
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        let audit = f.audit();
        let entries: Vec<serde_json::Value> = audit
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(entries.iter().all(|e| e["profile_id"] == "account-a"));
        assert!(
            entries
                .iter()
                .all(|e| e["target"]["repo"] == "sandbox/widget")
        );
    }

    #[test]
    fn identity_cross_repo_reads_ignore_checkout_repo_but_repo_commands_infer_it() {
        let f = cross_repo_fixture(&single_profile_policy());
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(f.home.path())
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
        };
        git(&["init", "-b", "main"]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/upstream/unmapped.git",
        ]);
        let out = f
            .bare_command()
            .current_dir(f.home.path())
            .env("FORGE_CLI_INBOX_NO_CACHE", "1")
            .args(["--provider", "github", "inbox", "list"])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        git(&[
            "remote",
            "set-url",
            "origin",
            "https://github.com/sandbox/widget.git",
        ]);
        for args in [
            vec!["activity", "feed"],
            vec!["search", "issues", "example"],
            vec!["search", "prs", "example"],
            vec!["search", "refs-to", "1"],
        ] {
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
    }

    #[test]
    fn identity_cross_repo_reads_do_not_fallback_on_repo_credential_or_permission_refusal() {
        for (repo, missing, denied, actor, expected) in [
            (
                "upstream/unmapped",
                false,
                false,
                "account-a",
                "identity_repository_unknown",
            ),
            (
                "sandbox/widget",
                true,
                false,
                "account-a",
                "identity_credential_missing",
            ),
            (
                "sandbox/widget",
                false,
                true,
                "account-a",
                "identity_operation_denied",
            ),
            (
                "sandbox/widget",
                false,
                false,
                "wrong-actor",
                "identity_actor_mismatch",
            ),
        ] {
            let policy = if denied {
                single_profile_policy().replace("operations = [\"api_read\",", "operations = [")
            } else {
                single_profile_policy()
            };
            let f = cross_repo_fixture(&policy);
            let mut command = f.bare_command();
            command
                .current_dir(f.home.path())
                .env("FORGE_CLI_INBOX_NO_CACHE", "1")
                .env("FIXTURE_ACTOR", actor);
            if missing {
                command.env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL");
            }
            let out = command
                .args(["--provider", "github", "--repo", repo, "inbox", "list"])
                .output()
                .unwrap();
            assert!(!out.status.success());
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(expected),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            assert!(!f.home.path().join("calls").exists());
        }
    }

    #[test]
    fn identity_inbox_canonicalizes_github_ssh_transport_alias() {
        let f = cross_repo_fixture(&single_profile_policy());
        for args in [
            vec!["init", "-b", "main"],
            vec![
                "remote",
                "add",
                "origin",
                "ssh://git@ssh.github.com:443/sandbox/widget.git",
            ],
        ] {
            assert!(
                Command::new("git")
                    .current_dir(f.home.path())
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        for repo in [None, Some("sandbox/widget")] {
            let mut command = f.bare_command();
            command
                .current_dir(f.home.path())
                .args(["--provider", "github"]);
            if let Some(repo) = repo {
                command.args(["--repo", repo]);
            }
            let out = command
                .args(["inbox", "list", "--no-cache"])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["data"]["providers"][0]["host"], "github.com");
        }
    }

    #[test]
    fn identity_inbox_does_not_read_or_write_unbound_cache() {
        let f = cross_repo_fixture(&format!(
            "activation='asserted-only'\n{}",
            single_profile_policy()
        ));
        let script = fs::read_to_string(&f.gh).unwrap().replace(
            "*) printf '[]';;",
            "*) printf '[{\"number\":1,\"url\":\"https://github.com/sandbox/widget/pull/1\",\"title\":\"Cached fixture item\"}]';;",
        );
        fs::write(&f.gh, script).unwrap();
        let cache = f.home.path().join("cache");
        let run = |missing: bool, managed: bool| {
            let mut command = f.bare_command();
            command
                .current_dir(f.home.path())
                .env("FORGE_CLI_INBOX_CACHE_DIR", &cache)
                .env_remove("FORGE_CLI_INBOX_NO_CACHE");
            if !managed {
                command
                    .env_remove("FORGE_IDENTITY_PRINCIPAL")
                    .env("GH_TOKEN", CANARY);
            }
            if missing {
                command.env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL");
            }
            command
                .args(["--provider", "github", "inbox", "list", "--cache-fallback"])
                .output()
                .unwrap()
        };
        let success = run(false, true);
        assert_eq!(success.status.code(), Some(0));
        assert!(
            !cache.exists(),
            "managed inbox must not populate the unbound cache"
        );
        // Seed the unbound cache through an ordinary invocation, then ensure an
        // identity refusal does not consume that other identity context.
        let seed = run(false, false);
        assert_eq!(seed.status.code(), Some(0));
        assert!(cache.exists());
        let refused = run(true, true);
        assert!(!refused.status.success());
        let value: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
        let provider = &value["error"]["details"]["providers"][0];
        assert_eq!(provider["error"]["kind"], "identity_credential_missing");
        assert!(provider.get("cache").is_none());
        assert_eq!(provider["item_count"], 0);
        assert!(!String::from_utf8_lossy(&refused.stdout).contains("provider_cache_fallback"));
    }

    #[test]
    fn identity_cross_repo_app_profile_verifies_app_and_viewer_without_repo_probe() {
        let policy = single_profile_policy().replacen(
            "expected_login = \"account-a\"",
            "expected_app_id=42\napp_slug='fixture-app'",
            1,
        );
        for (app_id, viewer, expected) in [
            ("42", "fixture-app[bot]", 0),
            ("43", "fixture-app[bot]", 1),
            ("42", "wrong-actor", 1),
        ] {
            let f = Fixture::new(&policy);
            fs::write(&f.gh, r#"#!/bin/sh
case "$1:$2" in
 api:apps/fixture-app) printf 'app\n' >> "$FIXTURE_PROBES"; printf '{"id":%s}' "$FIXTURE_APP_ID"; exit 0;;
 api:graphql) printf 'viewer\n' >> "$FIXTURE_PROBES"; printf '{"data":{"viewer":{"login":"%s"}}}' "$FIXTURE_VIEWER"; exit 0;;
 api:installation/repositories*) printf 'repository\n' >> "$FIXTURE_PROBES"; exit 9;;
esac
printf '[]'
"#).unwrap();
            let probes = f.home.path().join("probes");
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .env("FIXTURE_PROBES", &probes)
                .env("FIXTURE_APP_ID", app_id)
                .env("FIXTURE_VIEWER", viewer)
                .args(["--provider", "github", "inbox", "list", "--no-cache"])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(expected),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            let calls = fs::read_to_string(probes).unwrap();
            assert!(calls.contains("app"));
            assert!(!calls.contains("repository"));
            if app_id == "42" {
                assert!(calls.contains("viewer"));
            }
            if expected != 0 {
                assert!(String::from_utf8_lossy(&out.stdout).contains("identity_actor_mismatch"));
            }
        }
    }

    #[test]
    fn identity_repo_view_commit_read_binds_host_and_principal() {
        let f = cross_repo_fixture(&POLICY.replace("github.com", "enterprise.example"));
        fs::write(&f.gh, r#"#!/bin/sh
[ "$GH_HOST" = enterprise.example ] || exit 96
[ "$GH_ENTERPRISE_TOKEN" = "$FIXTURE_ACCOUNT_A_CREDENTIAL" ] || exit 95
[ -z "$GH_TOKEN" ] || exit 94
case "$1:$2" in
 api:user) printf '{"login":"account-a"}'; exit 0;;
esac
printf 'read\n' >> "$FIXTURE_CALL_LOG"
case "$*" in
 'repo view enterprise.example/sandbox/widget --json name,owner,defaultBranchRef,mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed,url') printf '%s' '{"owner":{"login":"sandbox"},"name":"widget","url":"https://enterprise.example/sandbox/widget","defaultBranchRef":{"name":"feature/example"}}';;
 'api --hostname enterprise.example repos/sandbox/widget/commits/feature%2Fexample') printf '%s' '{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}';;
 *) echo "unexpected argv: $*" >&2; exit 97;;
esac
"#).unwrap();
        let out = f
            .command()
            .current_dir(f.home.path())
            .args([
                "--provider",
                "github",
                "--host",
                "enterprise.example",
                "repo",
                "view",
            ])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            v["data"]["default_branch_head_sha"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            fs::read_to_string(f.home.path().join("calls"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert!(f.audit().lines().all(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            v["profile_id"] == "account-a"
                && v["target"]["host"] == "enterprise.example"
                && v["target"]["repo"] == "sandbox/widget"
        }));
    }

    #[test]
    fn identity_org_search_uses_cross_repo_profile_and_refuses_ambiguity() {
        for (policy, expected) in [(single_profile_policy(), 0), (POLICY.to_string(), 65)] {
            let f = cross_repo_fixture(&policy);
            let out = f
                .bare_command()
                .current_dir(f.home.path())
                .args([
                    "--provider",
                    "github",
                    "search",
                    "issues",
                    "org:sandbox org:example is:issue is:open",
                ])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(expected),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            if expected == 65 {
                assert!(String::from_utf8_lossy(&out.stdout).contains("identity_target_ambiguous"));
                assert!(!f.home.path().join("calls").exists());
            }
        }
    }

    #[test]
    fn identity_cross_repo_ambiguity_names_candidates_and_repo_recovery() {
        let f = cross_repo_fixture(POLICY);
        let out = f
            .bare_command()
            .current_dir(f.home.path())
            .args(["--provider", "github", "inbox", "list"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["error"]["code"], "identity_target_ambiguous");
        let message = String::from_utf8_lossy(&out.stdout);
        for expected in [
            "account-a",
            "account-b",
            "--repo",
            "FORGE_IDENTITY_PRINCIPAL",
        ] {
            assert!(message.contains(expected), "missing {expected}: {message}");
        }
        assert!(!f.home.path().join("calls").exists());
    }

    #[test]
    fn identity_asserted_only_api_passthrough_unset_and_strict_when_set() {
        let f = Fixture::new(&format!("activation='asserted-only'\n{POLICY}"));
        let glab = f.home.path().join("glab");
        fs::write(&glab,r#"#!/bin/sh
printf '{"iid":1,"web_url":"https://gitlab.example.invalid/example/project/-/issues/1","state":"opened","title":"Example","description":"Example","labels":[],"assignees":[]}'
"#).unwrap();
        fs::set_permissions(&glab, fs::Permissions::from_mode(0o700)).unwrap();
        for (provider, host, repo) in [
            ("github", "github.com", "upstream/unmapped"),
            ("gitlab", "gitlab.example.invalid", "example/project"),
        ] {
            let out = f
                .bare_command()
                .env_remove("FORGE_IDENTITY_PRINCIPAL")
                .env("FORGE_CLI_GLAB_BIN", &glab)
                .env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL")
                .env_remove("FIXTURE_SECRET_B")
                .env_remove("GH_TOKEN")
                .env_remove("GITHUB_TOKEN")
                .args([
                    "--provider",
                    provider,
                    "--host",
                    host,
                    "--repo",
                    repo,
                    "issue",
                    "view",
                    "1",
                ])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        let out = f.command().args(["identity", "explain"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["data"]["selection"]
                ["principal"],
            "contributor"
        );
        let out = f
            .bare_command()
            .args([
                "--provider",
                "github",
                "--repo",
                "upstream/unmapped",
                "identity",
                "explain",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        assert!(String::from_utf8_lossy(&out.stdout).contains("identity_repository_unknown"));
        let out = f
            .command()
            .env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL")
            .args(["issue", "view", "1"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        assert!(String::from_utf8_lossy(&out.stdout).contains("identity_credential_missing"));
    }
    #[test]
    fn identity_commit_diagnostics_share_authoring_remote_selection_and_explicit_override() {
        let policy = format!(
            "{POLICY}\n[[rules]]\nid='publish'\nprincipal='contributor'\nprofile='account-b'\nrepo='github.com/sandbox/publish'\n"
        );
        for selector in [
            "branch.main.pushRemote",
            "remote.pushDefault",
            "branch.main.remote",
            "sole",
        ] {
            let f = Fixture::new(&policy);
            let repo = tempfile::tempdir().unwrap();
            let git = |args: &[&str]| {
                let out = Command::new("git")
                    .current_dir(repo.path())
                    .args(args)
                    .output()
                    .unwrap();
                assert!(out.status.success());
            };
            git(&["init", "-b", "main"]);
            if selector != "sole" {
                git(&[
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/sandbox/widget.git",
                ]);
                git(&["config", selector, "publish"]);
            }
            git(&[
                "remote",
                "add",
                "publish",
                "https://github.com/sandbox/publish.git",
            ]);
            let out = f
                .bare_command()
                .current_dir(repo.path())
                .args(["identity", "explain", "--operation", "commit"])
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["data"]["selection"]["profile_id"], "account-b");
            let doctor = f
                .bare_command()
                .current_dir(repo.path())
                .env("FIXTURE_ACTOR", "account-b")
                .args(["identity", "doctor", "--operation", "commit"])
                .output()
                .unwrap();
            assert_eq!(doctor.status.code(), Some(65));
            assert!(
                String::from_utf8_lossy(&doctor.stdout).contains("identity_signing_key_missing")
            );
            let audit: Vec<serde_json::Value> = f
                .audit()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert!(audit.iter().all(|entry| entry["profile_id"] == "account-b"));
            if selector != "sole" {
                for spelling in [vec!["--remote", "origin"], vec!["--remote=origin"]] {
                    let out = f
                        .bare_command()
                        .current_dir(repo.path())
                        .args(spelling)
                        .args(["identity", "explain", "--operation", "commit"])
                        .output()
                        .unwrap();
                    assert_eq!(out.status.code(), Some(0));
                    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                    assert_eq!(value["data"]["selection"]["profile_id"], "account-a");
                }
            }
        }
    }
    #[test]
    fn identity_preparation_and_execution_share_the_callers_deadline_without_secret_output() {
        use forge_cli::backend::{BackendCall, BackendProgram, BackendRunner, ProcessRunner};
        use forge_cli::error::ForgeError;
        use nils_test_support::{EnvGuard, GlobalStateLock};
        use std::time::{Duration, Instant};
        let lock = GlobalStateLock::new();
        for (principal, script, timeout) in [
            (
                "contributor",
                "if test \"$2\" = user; then printf '%s' \"$GH_TOKEN\" >&2; sleep 2; printf '{\"login\":\"account-a\"}'; else printf '{}'; fi",
                75,
            ),
            (
                "coordinator",
                "if test \"$1\" = auth; then sleep 2; printf '%s' \"$FIXTURE_SECRET_B\"; else printf '{\"login\":\"account-b\"}'; fi",
                75,
            ),
            (
                "contributor",
                "if test \"$2\" = user; then sleep 0.1; printf '{\"login\":\"account-a\"}'; else sleep 0.1; printf '{}'; fi",
                150,
            ),
        ] {
            let f = Fixture::new(POLICY);
            fs::write(&f.gh, format!("#!/bin/sh\n{script}\n")).unwrap();
            let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", f.home.path().to_str().unwrap());
            let _state = EnvGuard::set(
                &lock,
                "XDG_STATE_HOME",
                f.home.path().join("state").to_str().unwrap(),
            );
            let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", principal);
            let _secret = EnvGuard::set(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL", CANARY);
            let _named_secret = EnvGuard::set(&lock, "FIXTURE_SECRET_B", CANARY);
            let _gh = EnvGuard::set(&lock, "FORGE_CLI_GH_BIN", f.gh.to_str().unwrap());
            let call =
                BackendCall::new(BackendProgram::Gh, ["api", "repos/sandbox/widget/issues/1"]);
            let started = Instant::now();
            let result =
                ProcessRunner.run_raw_with_timeout(&call, Some(Duration::from_millis(timeout)));
            assert!(
                started.elapsed() < Duration::from_millis(750),
                "identity preparation exceeded caller deadline"
            );
            let error = result.unwrap_err();
            assert!(matches!(
                &error,
                ForgeError::BackendUnavailable {
                    kind: "identity_probe_timeout" | "backend_timeout",
                    ..
                }
            ));
            assert!(!format!("{error:?}").contains(CANARY));
            assert!(!f.audit().contains(CANARY));
        }
    }
    #[test]
    fn identity_explicit_api_target_allows_no_checkout_but_refuses_failed_checkout_metadata() {
        let f = Fixture::new(POLICY);
        let out = f
            .command()
            .current_dir(f.home.path())
            .args(["--host", "github.com", "identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        fs::write(f.home.path().join(".git"), "gitdir: missing-checkout\n").unwrap();
        let out = f
            .command()
            .current_dir(f.home.path())
            .args(["--host", "github.com", "issue", "view", "1"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        assert!(String::from_utf8_lossy(&out.stdout).contains("identity_target_unknown"));
        assert!(!f.home.path().join("calls").exists());
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

#[cfg(unix)]
mod session_binding_tests {
    use super::{Command, resolve};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::{fs, os::unix::fs::PermissionsExt};
    const POLICY: &str = include_str!("../../../nils-common/tests/fixtures/identity/policy.toml");
    struct Fixture {
        home: tempfile::TempDir,
        broker: std::path::PathBuf,
    }
    impl Fixture {
        fn new(required: bool) -> Self {
            let home = tempfile::tempdir().unwrap();
            fs::create_dir(home.path().join("forge-cli")).unwrap();
            let policy = format!(
                "require_session_binding = {required}\n{POLICY}\n[[launch_rules]]\nid='ordinary'\ninitiator='operator'\nprincipal='contributor'\n[[launch_rules]]\nid='review'\ninitiator='operator'\nrole='reviewer'\nprincipal='coordinator'\n"
            );
            fs::write(home.path().join("forge-cli/identity.toml"), policy).unwrap();
            let broker = home.path().join("agent-session");
            fs::write(&broker, "#!/bin/sh\nprintf '%s' \"$FIXTURE_PROJECTION\"\n").unwrap();
            fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
            let gh = home.path().join("gh");
            fs::write(
                &gh,
                "#!/bin/sh\necho credential-probe-forbidden >&2\nexit 99\n",
            )
            .unwrap();
            fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).unwrap();
            Self { home, broker }
        }
        fn command(&self, role: Option<&str>, session: &str) -> Command {
            let mut cmd = Command::new(resolve("forge-cli"));
            cmd.current_dir(self.home.path()).args(["--format","json","--provider","github","--host","github.com","--repo","sandbox/widget"])
                .env("XDG_CONFIG_HOME",self.home.path()).env("XDG_STATE_HOME",self.home.path().join("state"))
                .env("FORGE_IDENTITY_AGENT_SESSION_BIN",&self.broker)
                .env("FORGE_CLI_GH_BIN",self.home.path().join("gh"))
                .env("AGENT_SESSION_ID",session).env("AGENT_SESSION_RUNTIME_ID","generation-a")
                .env_remove("FORGE_IDENTITY_PRINCIPAL").env_remove("FORGE_IDENTITY_SESSION")
                .env("FIXTURE_PROJECTION",json!({"schema_version":"cli.agent-session.broker-identity.v1","ok":true,"data":{
                    "schema_version":"agent-session.forge-binding.v1","session_id":session,"session_incarnation":"generation-a",
                    "session_created_at":"2026-10-01T00:00:00Z","root":{"machine":"launch-source","session_id":"root-session","session_created_at":"2026-10-01T00:00:00Z"},
                    "parent":null,"initiator":"operator","role":role
                }}).to_string());
            cmd
        }
    }
    fn value(out: &std::process::Output) -> Value {
        serde_json::from_slice(&out.stdout).unwrap()
    }
    #[test]
    fn identity_session_binding_selects_roles_concurrently_without_credentials_or_audit_writes() {
        let f = Fixture::new(true);
        let children: Vec<_> = [
            (None, "ordinary-session"),
            (Some("reviewer"), "review-session"),
        ]
        .into_iter()
        .map(|(role, id)| {
            f.command(role, id)
                .args(["identity", "explain"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
        for (child, principal) in children.into_iter().zip(["contributor", "coordinator"]) {
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(0));
            let v = value(&out);
            assert_eq!(v["data"]["selection"]["principal"], principal);
            assert_eq!(
                v["data"]["selection"]["session_binding"]["initiator"],
                "operator"
            );
            assert_eq!(
                v["data"]["selection"]["session_binding"]["root"]["session_id"],
                "root-session"
            );
        }
        assert!(!f.home.path().join("state").exists());
    }
    #[test]
    fn identity_session_binding_mismatch_refuses_before_credential_probe_and_records_audit() {
        let f = Fixture::new(true);
        let out = f
            .command(Some("reviewer"), "review-session")
            .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
            .args(["issue", "view", "1"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(65));
        assert_eq!(
            value(&out)["error"]["code"],
            "identity_session_principal_mismatch"
        );
        assert!(!String::from_utf8_lossy(&out.stderr).contains("credential-probe-forbidden"));
        let audit =
            fs::read_to_string(f.home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
        assert!(audit.contains("identity_session_principal_mismatch"));
        assert!(audit.contains("review-session"));
        assert!(audit.contains("reviewer"));
        let record: Value = serde_json::from_str(audit.lines().last().unwrap()).unwrap();
        assert_eq!(record["principal"], "coordinator");
        assert_eq!(record["asserted_principal"], "contributor");
        assert_eq!(record["matched_launch_rule"], "review");
    }
    #[test]
    fn identity_session_binding_concurrent_refusals_keep_distinct_audit_provenance() {
        let f = Fixture::new(true);
        let children: Vec<_> = [
            (None, "ordinary-session", "coordinator"),
            (Some("reviewer"), "review-session", "contributor"),
        ]
        .into_iter()
        .map(|(role, id, assertion)| {
            f.command(role, id)
                .env("FORGE_IDENTITY_PRINCIPAL", assertion)
                .args(["issue", "view", "1"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
        for child in children {
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                value(&out)["error"]["code"],
                "identity_session_principal_mismatch"
            );
            assert!(!String::from_utf8_lossy(&out.stderr).contains("credential-probe-forbidden"));
        }
        let text =
            fs::read_to_string(f.home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
        let records: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // Existing API execution and command-refusal layers each retain a record.
        assert_eq!(records.len(), 4);
        for (id, principal) in [
            ("ordinary-session", "contributor"),
            ("review-session", "coordinator"),
        ] {
            let matching: Vec<_> = records.iter().filter(|r| r["session"] == id).collect();
            assert_eq!(matching.len(), 2);
            for record in matching {
                assert_eq!(record["principal"], principal);
                assert_eq!(record["session_binding"]["session_id"], id);
            }
        }
    }
    #[test]
    fn identity_session_binding_missing_stale_or_ambiguous_refuses() {
        let f = Fixture::new(true);
        for (projection, code) in [
            (json!({"ok":false}), "identity_session_binding_unavailable"),
            (
                json!({"schema_version":"cli.agent-session.broker-identity.v1","ok":true,"data":{
                    "schema_version":"agent-session.forge-binding.v1","session_id":"other-session","session_incarnation":"generation-a",
                    "session_created_at":"2026-10-01T00:00:00Z","root":{"machine":"launch-source","session_id":"root-session","session_created_at":"2026-10-01T00:00:00Z"},"parent":null,"initiator":"operator","role":null
                }}),
                "identity_session_binding_mismatch",
            ),
        ] {
            let out = f
                .command(None, "ordinary-session")
                .env("FIXTURE_PROJECTION", projection.to_string())
                .args(["identity", "explain"])
                .output()
                .unwrap();
            assert_eq!(value(&out)["error"]["code"], code);
        }
        let path = f.home.path().join("forge-cli/identity.toml");
        let policy = fs::read_to_string(&path).unwrap();
        fs::write(path,format!("{policy}\n[[launch_rules]]\nid='duplicate'\ninitiator='operator'\nrole='reviewer'\nprincipal='coordinator'\n")).unwrap();
        let out = f
            .command(Some("reviewer"), "review-session")
            .args(["identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(
            value(&out)["error"]["code"],
            "identity_launch_rule_ambiguous"
        );
    }
    #[test]
    fn identity_session_binding_required_cannot_be_disabled_by_unset_assertion() {
        let f = Fixture::new(true);
        let path = f.home.path().join("forge-cli/identity.toml");
        let policy = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("activation='asserted-only'\n{policy}")).unwrap();
        let out = f
            .command(Some("unmapped-role"), "ordinary-session")
            .args(["identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(value(&out)["data"]["selection"]["principal"], "contributor");
        assert_eq!(
            value(&out)["data"]["selection"]["session_binding"]["matched_launch_rule"],
            "ordinary"
        );
        let out = f
            .command(None, "ordinary-session")
            .env_remove("AGENT_SESSION_ID")
            .args(["identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(
            value(&out)["error"]["code"],
            "identity_session_binding_missing"
        );
    }
    #[test]
    fn identity_session_binding_opt_out_and_missing_policy_preserve_phase_one() {
        let f = Fixture::new(false);
        let out = f
            .command(Some("reviewer"), "review-session")
            .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
            .env("FORGE_IDENTITY_AGENT_SESSION_BIN", "missing-broker")
            .args(["identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(value(&out)["data"]["selection"]["principal"], "contributor");
        fs::remove_file(f.home.path().join("forge-cli/identity.toml")).unwrap();
        let out = f
            .command(None, "ordinary-session")
            .args(["identity", "explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(value(&out)["data"]["enforced"], false);
    }
}
