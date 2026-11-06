//! Real local Git operations exercise the environment prepared for pinned HTTPS.
use nils_common::forge_identity::prepare_git;
use nils_test_support::git::{git, init_repo_main_with_initial_commit};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn pinned_https_children_cannot_observe_token_and_local_transport_succeeds() {
    let lock = GlobalStateLock::new();
    let home = tempfile::tempdir().unwrap();
    let _git_environment: Vec<_> = std::env::vars_os()
        .filter_map(|(key, _)| {
            key.to_str()
                .filter(|key| key.starts_with("GIT_"))
                .map(str::to_owned)
        })
        .map(|key| EnvGuard::remove(&lock, &key))
        .collect();
    let _global = EnvGuard::set(&lock, "GIT_CONFIG_GLOBAL", "/dev/null");
    let _system = EnvGuard::set(&lock, "GIT_CONFIG_NOSYSTEM", "1");
    let repo = init_repo_main_with_initial_commit();
    let remote = home.path().join("remote.git");
    git(home.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/sandbox/widget.git",
        ],
    );
    fs::create_dir_all(home.path().join("forge-cli")).unwrap();
    fs::write(
        home.path().join("forge-cli/identity.toml"),
        include_str!("../fixtures/identity/policy.toml"),
    )
    .unwrap();
    let global = home.path().join("global.gitconfig");
    git(
        repo.path(),
        &[
            "config",
            "--file",
            global.to_str().unwrap(),
            "credential.helper",
            "!f() { cat >/dev/null; test \"$1\" = get && printf 'username=account-a\\npassword=FIXTURE_CHILD_CANARY\\n'; }; f",
        ],
    );
    let _global = EnvGuard::set(&lock, "GIT_CONFIG_GLOBAL", global.to_str().unwrap());
    let bins = StubBinDir::new();
    bins.write_exe("gh", "#!/bin/sh\nprintf '{\"login\":\"account-a\"}'\n");
    let _path = prepend_path(&lock, bins.path());
    let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
    let _state = EnvGuard::set(
        &lock,
        "XDG_STATE_HOME",
        home.path().join("state").to_str().unwrap(),
    );
    let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
    let hook_marker = home.path().join("hook-observed");
    let monitor_marker = home.path().join("monitor-observed");
    let hook = repo.path().join(".git/hooks/pre-push");
    let monitor = home.path().join("fsmonitor");
    for (script, marker) in [(&hook, &hook_marker), (&monitor, &monitor_marker)] {
        fs::write(script, format!("#!/bin/sh\nif test \"$FORGE_IDENTITY_TOKEN\" = FIXTURE_CHILD_CANARY; then printf observed > '{}'; fi\nexit 0\n", marker.display())).unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(
        repo.path(),
        &["config", "core.fsmonitor", monitor.to_str().unwrap()],
    );
    let expected_head = git(repo.path(), &["rev-parse", "HEAD"]);
    for operation in ["push", "fetch"] {
        // Substitute only the destination after preparing the governed operation:
        // the executed transport uses a local bare repository and a fake token.
        let mut transport = Command::new("git");
        transport.current_dir(repo.path());
        let authorization =
            prepare_git(&mut transport, Some(repo.path()), &[operation, "origin"]).unwrap();
        assert!(authorization.is_some());
        // Force an index read under the same pinned child environment. Git's
        // fetch/push index use varies with version and repository contents.
        let mut index = Command::new("git");
        index
            .current_dir(repo.path())
            .args(["status", "--porcelain"]);
        for (key, value) in transport.get_envs() {
            match value {
                Some(value) => {
                    index.env(key, value);
                }
                None => {
                    index.env_remove(key);
                }
            }
        }
        assert!(index.output().unwrap().status.success());
        let refspec = if operation == "push" {
            "HEAD:refs/heads/fixture"
        } else {
            "refs/heads/fixture"
        };
        let output = transport
            .args([operation, remote.to_str().unwrap(), refspec])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            git(
                repo.path(),
                &[
                    "rev-parse",
                    if operation == "push" {
                        "HEAD"
                    } else {
                        "FETCH_HEAD"
                    }
                ]
            ),
            expected_head
        );
    }
    // A single assertion reports both independent exposures in the RED run.
    assert_eq!(
        (hook_marker.exists(), monitor_marker.exists()),
        (false, false),
        "pinned token reached a hook or fsmonitor"
    );
    assert_eq!(
        git(
            home.path(),
            &[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/fixture"
            ]
        ),
        expected_head
    );
}
