//! Scoped network configuration for credential-pinned commands; no network I/O.
use nils_common::forge_identity::prepare_git;
use nils_test_support::git::{git, init_repo_main_with_initial_commit};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::{fs, process::Command};

const URL: &str = "https://github.com/sandbox/widget.git";

fn scenario(setting: &str, scope: &str, url_scoped: bool, ssh: bool, value: &str) {
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
            if ssh {
                "git@github.com:sandbox/widget.git"
            } else {
                URL
            },
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
            "!f() { cat >/dev/null; test \"$1\" = get && printf 'username=account-a\\npassword=FIXTURE_NETWORK_CANARY\\n'; }; f",
        ],
    );
    let _global = EnvGuard::set(&lock, "GIT_CONFIG_GLOBAL", global.to_str().unwrap());
    let system = home.path().join("system.gitconfig");
    let _system_file = EnvGuard::set(&lock, "GIT_CONFIG_SYSTEM", system.to_str().unwrap());
    let _system_enabled = EnvGuard::remove(&lock, "GIT_CONFIG_NOSYSTEM");
    let included = home.path().join("included.gitconfig");
    let key = if url_scoped {
        format!("http.https://github.com/.{setting}")
    } else {
        format!("http.{setting}")
    };
    let mut config = vec!["config"];
    match scope {
        "global" => config.extend(["--file", global.to_str().unwrap()]),
        "system" => config.extend(["--file", system.to_str().unwrap()]),
        "include" => {
            git(
                repo.path(),
                &["config", "include.path", included.to_str().unwrap()],
            );
            config.extend(["--file", included.to_str().unwrap()]);
        }
        "worktree" => {
            git(
                repo.path(),
                &["config", "extensions.worktreeConfig", "true"],
            );
            config.push("--worktree");
        }
        "local" => config.push("--local"),
        _ => unreachable!(),
    }
    config.extend([&key, value]);
    git(repo.path(), &config);
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
    let _profile_token = EnvGuard::set(
        &lock,
        "FIXTURE_ACCOUNT_A_CREDENTIAL",
        "FIXTURE_NETWORK_CANARY",
    );
    let trusted = matches!(scope, "global" | "system");
    for operation in ["push", "fetch"] {
        let mut transport = Command::new("git");
        transport.current_dir(repo.path());
        let result = prepare_git(&mut transport, Some(repo.path()), &[operation, "origin"]);
        if !trusted && let Err(error) = result {
            assert_eq!(error.code, "identity_transport_config_untrusted");
            assert!(error.to_string().contains("system/global"), "{error}");
            assert!(!error.to_string().contains(value) || value.is_empty());
            continue;
        }
        assert!(result.unwrap().is_some());
        let mut effective = Command::new("git");
        effective.current_dir(repo.path()).args([
            "config",
            "--get-urlmatch",
            &format!("http.{setting}"),
            URL,
        ]);
        for (key, value) in transport.get_envs() {
            match value {
                Some(value) => {
                    effective.env(key, value);
                }
                None => {
                    effective.env_remove(key);
                }
            }
        }
        let output = effective.output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), value);
        // Only the destination changes after preparation. Git's real transport
        // stays local, while its prepared config still contains the setting.
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
        assert!(
            trusted,
            "pinned {operation} honored {scope} {key}; local bare transport succeeded"
        );
    }
}

#[test]
fn pinned_https_refuses_repository_proxy() {
    untrusted("proxy", "http://proxy.invalid:8080");
}

#[test]
fn pinned_https_refuses_repository_ca_info() {
    untrusted("sslCAInfo", "fixture-ca.pem");
}

#[test]
fn pinned_https_refuses_repository_ca_path() {
    untrusted("sslCAPath", "fixture-ca-directory");
}

fn untrusted(setting: &str, value: &str) {
    for scope in ["local", "worktree", "include"] {
        for url_scoped in [false, true] {
            for ssh in [false, true] {
                scenario(setting, scope, url_scoped, ssh, value);
            }
        }
    }
    // Empty entries reset inherited configuration and are still untrusted.
    scenario(setting, "local", false, false, "");
}

#[test]
fn pinned_https_preserves_global_and_system_network_configuration() {
    for scope in ["global", "system"] {
        for (setting, value) in [
            ("proxy", "http://proxy.invalid:8080"),
            ("sslCAInfo", "fixture-ca.pem"),
            ("sslCAPath", "fixture-ca-directory"),
        ] {
            for url_scoped in [false, true] {
                scenario(setting, scope, url_scoped, false, value);
            }
        }
    }
}
