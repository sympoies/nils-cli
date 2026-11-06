use crate::common::{GitCliHarness, git, init_repo};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::Value;

#[test]
fn transport_switch_and_status_round_trip() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:example/project.git",
        ],
    );
    for (transport, url) in [
        ("https", "https://github.com/example/project.git"),
        ("ssh", "git@github.com:example/project.git"),
    ] {
        let set = harness.run(
            repo.path(),
            &["transport", "set", transport, "--format", "json"],
        );
        assert_eq!(set.code, 0, "{}", set.stderr_text());
        let status = harness.run(repo.path(), &["transport", "status", "--format", "json"]);
        assert_eq!(status.code, 0, "{}", status.stderr_text());
        let json: Value = serde_json::from_str(&status.stdout_text()).unwrap();
        assert_eq!(json["schema_version"], "cli.git-cli.transport.status.v1");
        assert_eq!(json["data"]["transport"], transport);
        assert_eq!(json["data"]["url"], url);
        assert_eq!(json["data"]["recorded_in"], "remote.origin.url");
        assert_eq!(
            git(
                repo.path(),
                &["config", "--local", "--get", "remote.origin.url"]
            )
            .trim(),
            url
        );
    }
}

#[test]
fn governed_https_fetch_reports_account_mismatch_and_fix_hint() {
    use nils_test_support::{StubBinDir, cmd::run_with};
    use std::{fs, process::Command};

    let harness = GitCliHarness::new();
    let repo = init_repo();
    let config = tempfile::tempdir().unwrap();
    let stubs = StubBinDir::new();
    let actual_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let actual_git = String::from_utf8(actual_git.stdout).unwrap();
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/sandbox/widget.git",
        ],
    );
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(
        repo.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    fs::create_dir_all(config.path().join("forge-cli")).unwrap();
    fs::write(
        config.path().join("forge-cli/identity.toml"),
        include_str!("../../../nils-common/tests/fixtures/identity/policy.toml"),
    )
    .unwrap();
    let global = config.path().join("global.gitconfig");
    git(
        repo.path(),
        &[
            "config",
            "--file",
            global.to_str().unwrap(),
            "credential.helper",
            "",
        ],
    );
    git(
        repo.path(),
        &[
            "config",
            "--file",
            global.to_str().unwrap(),
            "--add",
            "credential.helper",
            "!f() { test \"$1\" = get || exit 0; cat >/dev/null; printf 'username=fixture\\npassword=%s\\n' \"$CLI_HELPER_SECRET\"; }; f",
        ],
    );
    stubs.write_exe("gh", "#!/bin/sh\nprintf '%s' '{\"login\":\"account-b\"}'\n");
    stubs.write_exe("git", &format!("#!/bin/sh\ncase \"$1\" in fetch|push|ls-remote) printf attempted > \"$CLI_TRANSPORT_MARKER\"; exit 99;; *) exec '{}' \"$@\";; esac\n", actual_git.trim()));
    let marker = config.path().join("transport-attempted");
    let options = harness
        .cmd_options(repo.path())
        .with_path_prepend(stubs.path())
        .with_env("XDG_CONFIG_HOME", config.path().to_str().unwrap())
        .with_env("GIT_CONFIG_GLOBAL", global.to_str().unwrap())
        .with_env("FORGE_IDENTITY_PRINCIPAL", "contributor")
        .with_env("CLI_HELPER_SECRET", "FIXTURE_CLI_HELPER_CANARY")
        .with_env("CLI_TRANSPORT_MARKER", marker.to_str().unwrap());
    let output = run_with(
        &harness.git_cli_bin(),
        &["sync-default", "--format", "json"],
        &options,
    );
    assert_ne!(output.code, 0);
    assert!(
        !marker.exists(),
        "account refusal must precede Git transport"
    );
    let rendered = output.stdout_text();
    assert!(rendered.contains("identity_actor_mismatch"), "{rendered}");
    assert!(
        rendered.contains("configure the credential helper"),
        "{rendered}"
    );
    assert!(!rendered.contains("FIXTURE_CLI_HELPER_CANARY"));
}
