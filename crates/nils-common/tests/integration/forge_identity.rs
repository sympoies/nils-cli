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
    let mut text = FIXTURE.to_string();
    text.push_str(&format!("\n[[rules]]\nid='managed'\nprincipal='contributor'\npath={}\nrepositories=['github.com/sandbox/widget','github.com/other/widget']\nprofile='account-b'\n", toml::Value::String(path.path().to_str().unwrap().into())));
    let p = Policy::parse(&text).unwrap();
    assert_eq!(
        p.resolve(
            "contributor",
            &target("sandbox/widget"),
            Some(path.path()),
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
            Some(path.path()),
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
            Some(path.path()),
            Operation::ApiRead
        )
        .unwrap_err()
        .code,
        "identity_path_conflict"
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
        for args in [
            vec![
                "push",
                "--repo=https://github.com/sandbox/other.git",
                "origin",
            ],
            vec!["fetch", "--multiple", "origin", "secondary"],
            vec!["fetch", "--upload-pack=alternate", "origin"],
        ] {
            let refused = identity::prepare_git(&mut Command::new("git"), Some(repo.path()), &args);
            assert_eq!(refused.err().map(|e| e.code), Some("identity_git_override"));
        }
        let mut cmd = Command::new("git");
        let auth = identity::prepare_git(
            &mut cmd,
            Some(repo.path()),
            &["push", "origin", "HEAD:refs/heads/fixture"],
        )
        .unwrap()
        .unwrap();
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
        let fixture = FIXTURE.replace("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", &fingerprint);
        install(home.path(), &fixture);
        raw_git(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/sandbox/widget.git",
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
        // Terminate this fixture's daemon before removing its private temporary keyring.
        let _ = Command::new("gpgconf")
            .args(["--kill", "gpg-agent"])
            .status();
    }
}
