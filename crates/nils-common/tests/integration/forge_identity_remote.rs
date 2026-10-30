//! Detached commit targeting preserves the identity owner's remote selection.
use nils_common::forge_identity::authoring_remote;
use nils_test_support::git::{git, init_repo_main_with_initial_commit};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

#[test]
fn detached_authoring_remote_preserves_defaults_ambiguity_and_probe_refusal() {
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
    let _home = EnvGuard::set(&lock, "HOME", home.path().to_str().unwrap());
    let _global = EnvGuard::set(&lock, "GIT_CONFIG_GLOBAL", "/dev/null");
    let _system = EnvGuard::set(&lock, "GIT_CONFIG_NOSYSTEM", "1");
    let repo = init_repo_main_with_initial_commit();
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/sandbox/widget.git",
        ],
    );
    git(repo.path(), &["checkout", "--detach"]);
    assert_eq!(authoring_remote(Some(repo.path())).unwrap(), "origin");
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/sandbox/other.git",
        ],
    );
    assert_eq!(
        authoring_remote(Some(repo.path())).unwrap_err().code,
        "identity_target_ambiguous"
    );
    git(repo.path(), &["config", "remote.pushDefault", "upstream"]);
    assert_eq!(authoring_remote(Some(repo.path())).unwrap(), "upstream");
    git(repo.path(), &["checkout", "main"]);
    git(repo.path(), &["config", "branch.main.remote", "origin"]);
    git(repo.path(), &["config", "branch.main.pushRemote", "origin"]);
    assert_eq!(authoring_remote(Some(repo.path())).unwrap(), "origin");
    git(
        repo.path(),
        &["config", "--unset", "branch.main.pushRemote"],
    );
    assert_eq!(authoring_remote(Some(repo.path())).unwrap(), "upstream");
    git(repo.path(), &["config", "--unset", "remote.pushDefault"]);
    assert_eq!(authoring_remote(Some(repo.path())).unwrap(), "origin");
    assert_eq!(
        authoring_remote(Some(home.path())).unwrap_err().code,
        "identity_target_unknown"
    );
}
