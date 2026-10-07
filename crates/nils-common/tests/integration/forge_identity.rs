use nils_common::forge_identity::{Operation, Policy, Target};
use pretty_assertions::assert_eq;
const FIXTURE: &str = include_str!("../fixtures/identity/policy.toml");
fn target(repo: &str) -> Target {
    Target::new("github.com", repo).unwrap()
}
#[test]
fn identity_two_principals_on_same_repo_and_repo_over_org_precedence() {
    let p = Policy::parse(FIXTURE).unwrap();
    let a = p
        .resolve(
            "contributor",
            &target("sandbox/widget"),
            None,
            Operation::Commit,
        )
        .unwrap();
    let b = p
        .resolve(
            "coordinator",
            &target("sandbox/widget"),
            None,
            Operation::Commit,
        )
        .unwrap();
    assert_eq!(a.profile_id, "account-a");
    assert_eq!(b.profile_id, "account-b");
    assert_eq!(a.matched_rule, "contributor-widget");
    assert_eq!(
        p.resolve(
            "contributor",
            &target("sandbox/other"),
            None,
            Operation::ApiRead
        )
        .unwrap()
        .matched_rule,
        "contributor-org"
    );
    assert_eq!(
        p.resolve(
            "coordinator",
            &target("sandbox/default"),
            None,
            Operation::ApiRead
        )
        .unwrap()
        .matched_rule,
        "principal-default"
    );
    assert_eq!(
        p.resolve(
            "coordinator",
            &target("sandbox/unknown"),
            None,
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_repository_unknown"
    );
    assert_eq!(
        p.resolve(
            "unknown",
            &target("sandbox/widget"),
            None,
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_principal_unknown"
    );
}
#[test]
fn identity_equal_precedence_conflicts_refuse() {
    let text = format!(
        "{FIXTURE}\n[[rules]]\nid='duplicate'\nprincipal='contributor'\nrepo='github.com/sandbox/widget'\nprofile='account-b'\n"
    );
    let p = Policy::parse(&text).unwrap();
    assert_eq!(
        p.resolve(
            "contributor",
            &target("sandbox/widget"),
            None,
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_rule_ambiguous"
    );
}
#[test]
fn identity_path_remote_conflict_refuses_and_canonical_path_fallback_is_allowlisted() {
    let path = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(path.path()).unwrap();
    let mut text = FIXTURE.to_string();
    text.push_str(&format!("\n[[rules]]\nid='managed'\nprincipal='contributor'\npath={}\nrepositories=['github.com/sandbox/widget','github.com/other/widget']\nprofile='account-b'\n", toml::Value::String(canonical.to_str().unwrap().into())));
    let p = Policy::parse(&text).unwrap();
    assert_eq!(
        p.resolve(
            "contributor",
            &target("sandbox/widget"),
            Some(&canonical),
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_path_conflict"
    );
    assert_eq!(
        p.resolve(
            "contributor",
            &target("other/widget"),
            Some(&canonical),
            Operation::ApiRead
        )
        .unwrap()
        .matched_rule,
        "managed"
    );
    assert_eq!(
        p.resolve(
            "contributor",
            &target("other/unknown"),
            Some(&canonical),
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_path_conflict"
    );
}
#[cfg(unix)]
#[test]
fn identity_symlink_path_selector_is_refused_before_it_can_skip_a_conflict() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let policy = format!(
        "{FIXTURE}\n[[rules]]\nid='managed-alias'\nprincipal='contributor'\npath={}\nrepositories=['github.com/sandbox/widget']\nprofile='account-b'\n",
        toml::Value::String(alias.to_str().unwrap().into())
    );
    assert_eq!(
        Policy::parse(&policy).err().map(|e| e.code),
        Some("identity_policy_invalid")
    );
}
#[test]
fn identity_strict_policy_errors_never_echo_source_canary() {
    for bad in [
        FIXTURE.replace("version = 1", "version = 2"),
        FIXTURE.replace("version = 1", "version = 1\ntoken='CANARY_NEVER_OUTPUT_07'"),
    ] {
        let message = Policy::parse(&bad).unwrap_err().to_string();
        assert!(!message.contains("CANARY_NEVER_OUTPUT_07"));
    }
    assert!(
        Policy::parse(&FIXTURE.replace("profiles = [\"account-b\"]", "profiles = [\"missing\"]"))
            .is_err()
    );
    assert!(
        Policy::parse(&FIXTURE.replace(
            "repo = \"github.com/sandbox/widget\"",
            "repo = \"github.com/sandbox/widget\"\norg = \"github.com/sandbox\""
        ))
        .is_err()
    );
    assert!(Target::new("github.com:443", "sandbox/widget").is_err());
    assert!(Target::new("github.com", "sandbox/../widget").is_err());
}

#[test]
fn gitlab_targets_allow_nested_groups_but_github_targets_remain_two_components() {
    assert_eq!(
        Policy::parse(&FIXTURE.replacen(
            "version = 1",
            "version = 1\ngitlab_hosts = ['GitLab.example.invalid']",
            1,
        ))
        .unwrap_err()
        .code,
        "identity_policy_invalid"
    );
    assert_eq!(
        Target::new_gitlab("gitlab.example.invalid", "group/subgroup/project")
            .unwrap()
            .repo,
        "group/subgroup/project"
    );
    assert!(Target::new("github.com", "group/subgroup/project").is_err());
    assert_eq!(
        Target::new_gitlab("altssh.gitlab.com", "group/subgroup/project")
            .unwrap()
            .host,
        "gitlab.com"
    );
    let policy = Policy::parse(&format!(
        "{}\n\n[[rules]]\nid='gitlab-project'\nprincipal='contributor'\nrepo='gitlab.example.invalid/group/subgroup/project'\nprofile='account-a'\n",
        FIXTURE.replacen("version = 1", "version = 1\ngitlab_hosts = ['gitlab.example.invalid']", 1)
    )).unwrap();
    assert_eq!(
        policy
            .resolve(
                "contributor",
                &Target::new_gitlab("gitlab.example.invalid", "group/subgroup/project").unwrap(),
                None,
                Operation::Commit,
            )
            .unwrap()
            .matched_rule,
        "gitlab-project"
    );
}

#[test]
fn nested_gitlab_organization_rules_match_only_their_namespace() {
    let text = FIXTURE
        .replacen(
            "version = 1",
            "version = 1\ngitlab_hosts = ['gitlab.example.invalid']",
            1,
        )
        .replace(
            "repo = \"github.com/sandbox/widget\"",
            "org = \"gitlab.example.invalid/group/subgroup\"",
        );
    let policy = Policy::parse(&text).unwrap();
    let matching = Target::new_gitlab("gitlab.example.invalid", "group/subgroup/project").unwrap();
    assert_eq!(
        policy
            .resolve("contributor", &matching, None, Operation::Commit)
            .unwrap()
            .matched_rule,
        "contributor-widget"
    );
    let sibling = Target::new_gitlab("gitlab.example.invalid", "group/other/project").unwrap();
    assert_eq!(
        policy
            .resolve("contributor", &sibling, None, Operation::Commit)
            .unwrap_err()
            .code,
        "identity_repository_unknown"
    );
}
#[test]
fn identity_operation_allowlist_does_not_fallback_to_org_profile() {
    let p = Policy::parse(&FIXTURE.replacen(
        "operations = [\"api_read\", \"api_write\", \"git_read\", \"git_push\", \"commit\"]",
        "operations = ['api_read']",
        1,
    ))
    .unwrap();
    assert_eq!(
        p.resolve(
            "contributor",
            &target("sandbox/widget"),
            None,
            Operation::GitPush
        )
        .unwrap_err()
        .code,
        "identity_operation_denied"
    );
}

#[cfg(unix)]
mod execution {
    use super::{FIXTURE, Operation, Policy, target};
    use nils_common::{forge_identity as identity, git};
    use nils_test_support::git::{git as raw_git, init_repo_main as init_repo};
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
    use pretty_assertions::assert_eq;
    use std::{fs, process::Command};
    const CANARY: &str = "FIXTURE_CANARY_SECRET_42";
    fn install(home: &std::path::Path, policy: &str) {
        fs::create_dir_all(home.join("forge-cli")).unwrap();
        fs::write(home.join("forge-cli/identity.toml"), policy).unwrap();
    }
    #[test]
    fn identity_session_binding_git_refuses_mismatch_before_any_credential_probe() {
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        let repo = init_repo();
        let bins = StubBinDir::new();
        bins.write_exe(
            "agent-session",
            "#!/bin/sh\nprintf '%s' \"$FIXTURE_PROJECTION\"\n",
        );
        bins.write_exe(
            "gh",
            "#!/bin/sh\necho credential-probe-forbidden >&2\nexit 99\n",
        );
        install(
            home.path(),
            &format!(
                "require_session_binding=true\n{FIXTURE}\n[[launch_rules]]\nid='bound-default'\ninitiator='operator'\nprincipal='coordinator'\n"
            ),
        );
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/sandbox/widget.git",
            ],
        );
        let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
        let _state = EnvGuard::set(
            &lock,
            "XDG_STATE_HOME",
            home.path().join("state").to_str().unwrap(),
        );
        let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        let _session = EnvGuard::set(&lock, "AGENT_SESSION_ID", "bound-session");
        let _runtime = EnvGuard::set(&lock, "AGENT_SESSION_RUNTIME_ID", "generation-a");
        let _bin = EnvGuard::remove(&lock, "FORGE_IDENTITY_AGENT_SESSION_BIN");
        let _path = prepend_path(&lock, bins.path());
        let projection=serde_json::json!({"schema_version":"cli.agent-session.broker-identity.v1","ok":true,"data":{
            "schema_version":"agent-session.forge-binding.v1","session_id":"bound-session","session_incarnation":"generation-a",
            "session_created_at":"2026-10-01T00:00:00Z","root":{"machine":"launch-source","session_id":"bound-session","session_created_at":"2026-10-01T00:00:00Z"},
            "parent":null,"initiator":"operator","role":null
        }}).to_string();
        let _projection = EnvGuard::set(&lock, "FIXTURE_PROJECTION", &projection);
        for args in [
            vec!["fetch", "origin"],
            vec!["push", "origin", "HEAD"],
            vec!["commit", "-m", "fixture"],
        ] {
            let err = git::run_output_in(repo.path(), &args).unwrap_err();
            assert!(
                err.to_string()
                    .contains("identity_session_principal_mismatch")
            );
        }
        let text =
            fs::read_to_string(home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
        for line in text.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["principal"], "coordinator");
            assert_eq!(record["session_binding"]["session_id"], "bound-session");
        }
    }
    #[test]
    fn identity_git_missing_credential_refuses_before_transport_and_local_reads_remain_available() {
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        let repo = init_repo();
        install(home.path(), FIXTURE);
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:sandbox/widget.git",
            ],
        );
        let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
        let _state = EnvGuard::set(
            &lock,
            "XDG_STATE_HOME",
            home.path().join("state").to_str().unwrap(),
        );
        let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        let _credential = EnvGuard::remove(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL");
        let _ambient = EnvGuard::set(&lock, "GH_TOKEN", CANARY);
        assert!(
            git::run_output_in(repo.path(), &["status", "--porcelain"])
                .unwrap()
                .status
                .success()
        );
        let err = git::run_output_in(repo.path(), &["push", "origin", "HEAD:refs/heads/fixture"])
            .unwrap_err();
        assert!(err.to_string().contains("identity_credential_missing"));
        assert!(!err.to_string().contains(CANARY));
        let audit =
            fs::read_to_string(home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
        assert!(audit.contains("identity_credential_missing"));
        assert!(!audit.contains(CANARY));
    }
    #[test]
    fn identity_selected_pushurl_multiple_urls_and_managed_worktree_paths() {
        let repo = init_repo();
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/sandbox/fetch.git",
            ],
        );
        raw_git(
            repo.path(),
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                "git@github.com:sandbox/widget.git",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", true)
                .unwrap()
                .0
                .repo,
            "sandbox/widget"
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", false)
                .unwrap()
                .0
                .repo,
            "sandbox/fetch"
        );
        raw_git(
            repo.path(),
            &[
                "remote",
                "set-url",
                "--add",
                "--push",
                "origin",
                "https://github.com/sandbox/other.git",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", true)
                .unwrap_err()
                .code,
            "identity_target_ambiguous"
        );
        raw_git(
            repo.path(),
            &[
                "config",
                "url.https://elsewhere.invalid/.insteadOf",
                "https://github.com/",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", false)
                .unwrap_err()
                .code,
            "identity_transport_override"
        );
        assert!(
            identity::target_for_remote(
                Some(repo.path()),
                "https://token@github.com/sandbox/widget",
                true
            )
            .is_err()
        );
    }
    #[test]
    fn target_for_remote_classifies_nested_gitlab_and_requires_self_host_configuration() {
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        let repo = init_repo();
        let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@GITLAB.COM:group/subgroup/project.git",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", false)
                .unwrap()
                .0
                .repo,
            "group/subgroup/project"
        );
        raw_git(
            repo.path(),
            &[
                "remote",
                "set-url",
                "origin",
                "ssh://git@altssh.gitlab.com/group/subgroup/project.git",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", false)
                .unwrap()
                .0
                .host,
            "gitlab.com"
        );
        raw_git(
            repo.path(),
            &[
                "remote",
                "set-url",
                "origin",
                "git@GITLAB.EXAMPLE.INVALID:group/subgroup/project.git",
            ],
        );
        assert_eq!(
            identity::target_for_remote(Some(repo.path()), "origin", false)
                .unwrap_err()
                .code,
            "identity_gitlab_host_not_configured_add_gitlab_hosts"
        );
    }
    #[test]
    fn identity_linked_worktree_uses_source_checkout_for_managed_path_rules() {
        let repo = nils_test_support::git::init_repo_main_with_initial_commit();
        let work = tempfile::tempdir().unwrap();
        let checkout = work.path().join("linked");
        nils_test_support::git::worktree_add_branch(repo.path(), &checkout, "feat/fixture");
        assert_eq!(
            identity::managed_path(Some(&checkout)).unwrap(),
            fs::canonicalize(repo.path()).unwrap()
        );
    }
    #[test]
    fn identity_transport_pins_helper_and_keeps_token_out_of_arguments() {
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        let repo = init_repo();
        let bins = StubBinDir::new();
        bins.write_exe("gh", "#!/bin/sh\nprintf '{\"login\":\"account-a\"}'\n");
        install(home.path(), FIXTURE);
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:sandbox/widget.git",
            ],
        );
        let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
        let _state = EnvGuard::set(
            &lock,
            "XDG_STATE_HOME",
            home.path().join("state").to_str().unwrap(),
        );
        let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        let _secret = EnvGuard::set(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL", CANARY);
        let _path = prepend_path(&lock, bins.path());
        let _tls = EnvGuard::set(&lock, "GIT_SSL_NO_VERIFY", "1");
        raw_git(repo.path(), &["config", "http.sslVerify", "false"]);
        raw_git(
            repo.path(),
            &[
                "config",
                "http.https://github.com/sandbox/widget.git.sslVerify",
                "false",
            ],
        );
        for args in [
            vec![
                "push",
                "--repo=https://github.com/sandbox/other.git",
                "origin",
            ],
            vec!["fetch", "--multiple", "origin", "secondary"],
            vec!["fetch", "--upload-pack=alternate", "origin"],
            vec!["-c", "push.followTags=false", "push", "origin"],
            vec!["-c", "push.recurseSubmodules=no", "push", "origin"],
            vec!["-c", "push.pushOption=ci.skip", "push", "origin"],
            vec!["-c", "push.default=nothing", "push", "origin"],
        ] {
            let refused = identity::prepare_git(&mut Command::new("git"), Some(repo.path()), &args);
            assert_eq!(refused.err().map(|e| e.code), Some("identity_git_override"));
        }
        let safe_empty_push_option = identity::prepare_git(
            &mut Command::new("git"),
            Some(repo.path()),
            &["-c", "push.pushOption=", "push", "origin"],
        );
        assert!(safe_empty_push_option.unwrap().is_some());
        let mut cmd = Command::new("git");
        let auth = identity::prepare_git(
            &mut cmd,
            Some(repo.path()),
            &["push", "origin", "HEAD:refs/heads/fixture"],
        )
        .unwrap()
        .unwrap();
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "GIT_SSL_NO_VERIFY" && v.is_none())
        );
        cmd.current_dir(repo.path()).args([
            "config",
            "--get-urlmatch",
            "http.sslVerify",
            "https://github.com/sandbox/widget.git",
        ]);
        let verification = cmd.output().unwrap();
        assert_eq!(String::from_utf8_lossy(&verification.stdout).trim(), "true");
        assert!(!format!("{:?}", cmd.get_args().collect::<Vec<_>>()).contains(CANARY));
        let env: std::collections::BTreeMap<_, _> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_str().unwrap().to_string(),
                        v.to_str().unwrap().to_string(),
                    )
                })
            })
            .collect();
        assert!(
            env.values()
                .any(|v| v == "url.https://github.com/sandbox/widget.git.insteadOf")
        );
        let mut bytes = CANARY.as_bytes().to_vec();
        auth.redact(&mut bytes);
        assert_eq!(bytes, b"[REDACTED]");
        let helper = env
            .iter()
            .find_map(|(k, v)| {
                (k.starts_with("GIT_CONFIG_VALUE_") && v.starts_with("!f()")).then_some(v)
            })
            .unwrap();
        let script = format!("{} get", helper.trim_start_matches('!'));
        use std::io::Write;
        for (host, path, allowed) in [
            ("github.com", "sandbox/widget.git", true),
            ("other.invalid", "sandbox/widget.git", false),
            ("github.com", "sandbox/other.git", false),
        ] {
            let mut child = Command::new("sh")
                .args(["-c", &script])
                .envs(&env)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("protocol=https\nhost={host}\npath={path}\n\n").as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).contains(CANARY),
                allowed
            );
        }
    }
    #[test]
    fn identity_commit_applies_author_committer_and_verified_signer() {
        if !nils_common::process::cmd_exists("gpg") {
            eprintln!("skip: gpg required for signed identity fixture");
            return;
        }
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        let repo = init_repo();
        let bins = StubBinDir::new();
        bins.write_exe("gh", "#!/bin/sh\nprintf '{\"login\":\"account-a\"}'\n");
        let keyhome = home.path().join("keyring");
        fs::create_dir(&keyhome).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&keyhome, fs::Permissions::from_mode(0o700)).unwrap();
        let _gpg = EnvGuard::set(&lock, "GNUPGHOME", keyhome.to_str().unwrap());
        struct FixtureAgent(std::path::PathBuf);
        impl Drop for FixtureAgent {
            fn drop(&mut self) {
                let _ = Command::new("gpgconf")
                    .env("GNUPGHOME", &self.0)
                    .args(["--kill", "gpg-agent"])
                    .output();
            }
        }
        let _agent = FixtureAgent(keyhome.clone());
        let generation = Command::new("gpg")
            .args([
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Example Contributor A <contributor-a@example.invalid>",
                "ed25519",
                "sign",
                "0",
            ])
            .output()
            .unwrap();
        assert!(
            generation.status.success(),
            "{}",
            String::from_utf8_lossy(&generation.stderr)
        );
        let key = Command::new("gpg")
            .args(["--batch", "--with-colons", "--list-secret-keys"])
            .output()
            .unwrap();
        let fingerprint = String::from_utf8_lossy(&key.stdout)
            .lines()
            .find_map(|line| {
                line.strip_prefix("fpr:")
                    .and_then(|_| line.split(':').nth(9))
            })
            .unwrap()
            .to_string();
        let second = Command::new("gpg")
            .args([
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Example Contributor B <contributor-b@example.invalid>",
                "ed25519",
                "sign",
                "0",
            ])
            .output()
            .unwrap();
        assert!(second.status.success());
        let second = Command::new("gpg")
            .args([
                "--batch",
                "--with-colons",
                "--list-secret-keys",
                "contributor-b@example.invalid",
            ])
            .output()
            .unwrap();
        let other_fingerprint = String::from_utf8_lossy(&second.stdout)
            .lines()
            .find_map(|line| {
                line.strip_prefix("fpr:")
                    .and_then(|_| line.split(':').nth(9))
            })
            .unwrap()
            .to_string();
        let fixture = format!(
            "{}\n\n[[rules]]\nid='nested-gitlab-project'\nprincipal='contributor'\nrepo='gitlab.example.invalid/group/subgroup/project'\nprofile='account-a'\n",
            FIXTURE
                .replace(
                    "version = 1",
                    "version = 1\ngitlab_hosts = ['gitlab.example.invalid']"
                )
                .replace("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", &fingerprint)
        );
        install(home.path(), &fixture);
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@GITLAB.EXAMPLE.INVALID:group/subgroup/project.git",
            ],
        );
        raw_git(
            repo.path(),
            &["config", "user.name", "Example Contributor A"],
        );
        raw_git(
            repo.path(),
            &["config", "user.email", "contributor-a@example.invalid"],
        );
        let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
        let _state = EnvGuard::set(
            &lock,
            "XDG_STATE_HOME",
            home.path().join("state").to_str().unwrap(),
        );
        let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        let _secret = EnvGuard::set(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL", CANARY);
        let _path = prepend_path(&lock, bins.path());
        let output = git::run_output_in(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: identity fixture"],
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata = raw_git(repo.path(), &["log", "-1", "--format=%an|%ae|%cn|%ce|%GF"]);
        assert_eq!(
            metadata.trim(),
            format!(
                "Example Contributor A|contributor-a@example.invalid|Example Contributor A|contributor-a@example.invalid|{fingerprint}"
            )
        );
        assert!(
            git::run_output_in(repo.path(), &["verify-commit", "HEAD"])
                .unwrap()
                .status
                .success()
        );
        for options in [
            vec![format!("--gpg-sign={other_fingerprint}")],
            vec![format!("-aS{other_fingerprint}")],
            vec![format!("--gpg-s={other_fingerprint}")],
            vec!["-C".into(), "HEAD".into()],
            vec!["-cHEAD".into()],
            vec!["--reuse-message=HEAD".into()],
            vec!["--reedit-message".into(), "HEAD".into()],
        ] {
            let mut args = vec!["commit"];
            args.extend(options.iter().map(String::as_str));
            let refused = identity::prepare_git(&mut Command::new("git"), Some(repo.path()), &args);
            assert_eq!(
                refused.err().map(|e| e.code),
                Some("identity_commit_override")
            );
        }
        if let Some(binary) =
            nils_test_support::bin::sibling_or_skip("semantic-commit", "nils-semantic-commit")
        {
            let out = Command::new(binary)
                .current_dir(repo.path())
                .args([
                    "commit",
                    "--message",
                    "feat: semantic identity fixture",
                    "--automation",
                    "--json",
                    "--quiet",
                    "--no-summary",
                    "--allow-empty",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                raw_git(repo.path(), &["log", "-1", "--format=%ae|%ce|%GF"]).trim(),
                format!(
                    "contributor-a@example.invalid|contributor-a@example.invalid|{fingerprint}"
                )
            );
        }
        let audit =
            fs::read_to_string(home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
        assert!(audit.contains("execution_succeeded"));
        assert!(audit.contains("\"object\""));
        assert!(!audit.contains(CANARY));
        raw_git(
            repo.path(),
            &["config", "user.email", "wrong@example.invalid"],
        );
        let err = git::run_output_in(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: refused fixture"],
        )
        .unwrap_err();
        assert!(err.to_string().contains("identity_commit_mismatch"));
        let mut p = Policy::parse(&fixture).unwrap();
        p.profiles.get_mut("account-a").unwrap().signing_fingerprint =
            "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC".into();
        assert_eq!(
            identity::verify_key(
                &p.resolve(
                    "contributor",
                    &target("sandbox/widget"),
                    None,
                    Operation::Commit
                )
                .unwrap()
                .profile
            )
            .unwrap_err()
            .code,
            "identity_signing_key_missing"
        );
        // FixtureAgent terminates only this temporary keyring's daemon, including on panic.
    }
}

#[test]
fn identity_asserted_only_unset_passthrough_and_asserted_strict_without_credentials() {
    use nils_common::forge_identity as identity;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use std::{fs, process::Command};
    let lock = GlobalStateLock::new();
    let home = tempfile::tempdir().unwrap();
    let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
    let _principal = EnvGuard::remove(&lock, "FORGE_IDENTITY_PRINCIPAL");
    fs::create_dir(home.path().join("forge-cli")).unwrap();
    let path = home.path().join("forge-cli/identity.toml");
    fs::write(&path, format!("activation='asserted-only'\n{FIXTURE}")).unwrap();
    assert!(identity::load().unwrap().is_none());
    // These are protected operations whose ordinary invocation/config must be
    // left intact, irrespective of provider, remote, repository or signing.
    for args in [
        vec![
            "fetch",
            "https://gitlab.example.invalid/example/project.git",
        ],
        vec!["push", "https://github.com/upstream/unmapped.git"],
        vec!["commit", "-m", "Example"],
    ] {
        let mut command = Command::new("git");
        assert!(
            identity::prepare_git(&mut command, Some(home.path()), &args)
                .unwrap()
                .is_none()
        );
        assert_eq!(command.get_envs().count(), 0);
        assert_eq!(command.get_args().count(), 0);
    }
    {
        let _assertion = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        let loaded = identity::load().unwrap().unwrap();
        assert_eq!(
            loaded
                .select(&target("sandbox/widget"), None, Operation::Commit)
                .unwrap()
                .profile_id,
            "account-a"
        );
        assert_eq!(
            loaded
                .select(&target("upstream/unmapped"), None, Operation::Commit)
                .unwrap_err()
                .code,
            "identity_repository_unknown"
        );
    }
    for assertion in ["", "unknown"] {
        let _assertion = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", assertion);
        let loaded = identity::load().unwrap().unwrap();
        assert_eq!(
            loaded
                .select(&target("sandbox/widget"), None, Operation::ApiRead)
                .unwrap_err()
                .code,
            "identity_principal_unknown"
        );
    }
    // Strict schema and the previous default remain active even with no assertion.
    fs::write(&path, FIXTURE).unwrap();
    assert_eq!(
        identity::load()
            .unwrap()
            .unwrap()
            .select(&target("sandbox/widget"), None, Operation::Commit)
            .unwrap_err()
            .code,
        "identity_principal_missing"
    );
    fs::write(
        &path,
        format!("activation='asserted-only'\nunknown='SOURCE_CANARY'\n{FIXTURE}"),
    )
    .unwrap();
    assert!(matches!(identity::load(),Err(e) if e.code=="identity_policy_invalid"));
    assert!(!home.path().join("state").exists());
}

#[test]
fn identity_cross_repository_scope_is_read_only_and_requires_one_profile() {
    let policy = Policy::parse(FIXTURE).unwrap();
    let target = Target::cross_repository("github.com").unwrap();
    let selected = policy
        .resolve("coordinator", &target, None, Operation::ApiRead)
        .unwrap();
    assert_eq!(selected.profile_id, "account-b");
    assert_eq!(selected.matched_rule, "principal-single-profile");
    assert_eq!(target.key(), "github.com");
    assert_eq!(
        serde_json::to_value(&target).unwrap(),
        serde_json::json!({"host": "github.com"})
    );
    for operation in [
        Operation::ApiWrite,
        Operation::GitRead,
        Operation::GitPush,
        Operation::Commit,
    ] {
        assert_eq!(
            policy
                .resolve("coordinator", &target, None, operation)
                .unwrap_err()
                .code,
            "identity_operation_denied"
        );
    }
    let error = policy
        .resolve("contributor", &target, None, Operation::ApiRead)
        .unwrap_err();
    assert_eq!(error.code, "identity_target_ambiguous");
    assert!(error.to_string().contains("account-a, account-b"));
    let denied =
        Policy::parse(&FIXTURE.replace("operations = [\"api_read\",", "operations = [")).unwrap();
    assert_eq!(
        denied
            .resolve("coordinator", &target, None, Operation::ApiRead)
            .unwrap_err()
            .code,
        "identity_operation_denied"
    );
    assert_eq!(
        Target::cross_repository("https://github.com")
            .unwrap_err()
            .code,
        "identity_target_invalid"
    );
}

#[test]
fn identity_cross_repository_scope_requires_a_principal_profile_host_declaration() {
    let target = Target::cross_repository("AUTHORIZED.EXAMPLE").unwrap();
    let base = FIXTURE.replace("github.com", "authorized.example")
        .replace("default_profile = \"account-b\"\ndefault_repositories = [\"authorized.example/sandbox/default\"]\n", "");
    let rule = "repo = \"authorized.example/sandbox/widget\"\nprofile = \"account-b\"";
    let path = tempfile::tempdir().unwrap();
    for declaration in [
        rule.to_string(),
        "org = \"authorized.example/sandbox\"\nprofile = \"account-b\"".to_string(),
        format!(
            "path = {:?}\nrepositories = [\"authorized.example/sandbox/widget\"]\nprofile = \"account-b\"",
            path.path().canonicalize().unwrap()
        ),
    ] {
        let policy = Policy::parse(&base.replace(rule, &declaration)).unwrap();
        assert_eq!(
            policy
                .resolve("coordinator", &target, None, Operation::ApiRead)
                .unwrap()
                .profile_id,
            "account-b"
        );
    }
    let default_only = Policy::parse(&FIXTURE.replace("github.com", "authorized.example")
        .replace("[[rules]]\nid = \"coordinator-widget\"\nprincipal = \"coordinator\"\nrepo = \"authorized.example/sandbox/widget\"\nprofile = \"account-b\"", "")).unwrap();
    assert!(
        default_only
            .resolve("coordinator", &target, None, Operation::ApiRead)
            .is_ok()
    );
    let foreign_principal = Policy::parse(&base.replace(
        "id = \"coordinator-widget\"\nprincipal = \"coordinator\"",
        "id = \"coordinator-widget\"\nprincipal = \"contributor\"",
    ))
    .unwrap();
    for (policy, host) in [
        (Policy::parse(FIXTURE).unwrap(), "unlisted.example"),
        (foreign_principal, "authorized.example"),
    ] {
        assert_eq!(
            policy
                .resolve(
                    "coordinator",
                    &Target::cross_repository(host).unwrap(),
                    None,
                    Operation::ApiRead
                )
                .unwrap_err()
                .code,
            "identity_repository_unknown"
        );
    }
}
