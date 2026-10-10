use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{self, run_resolved};
use nils_test_support::git::init_repo_main_with_initial_commit;
#[allow(unused_imports)]
pub use nils_test_support::git::{commit_file, git, repo_id};

#[allow(dead_code)]
pub fn init_repo() -> tempfile::TempDir {
    init_repo_main_with_initial_commit()
}

#[allow(dead_code)]
pub fn git_lock_bin() -> std::path::PathBuf {
    resolve("git-lock")
}

#[allow(dead_code)]
pub fn run_git_lock_output(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> Output {
    // Point forge identity and config lookups at an empty per-run directory, so
    // the developer's forge profiles cannot leak into the child process. Callers
    // may still override these through `envs`.
    let isolated = tempfile::TempDir::new().expect("isolated config dir");
    let xdg_config = isolated.path().join("xdg-config");
    let xdg_state = isolated.path().join("xdg-state");
    let mut all_envs = vec![
        ("XDG_CONFIG_HOME", xdg_config.to_str().expect("config path")),
        ("XDG_STATE_HOME", xdg_state.to_str().expect("state path")),
    ];
    all_envs.extend_from_slice(envs);
    let mut options =
        cmd::options_in_dir_with_envs(dir, &all_envs).with_env_remove("FORGE_IDENTITY_PRINCIPAL");
    options = match input {
        Some(text) => options.with_stdin_str(text),
        None => options.with_stdin_bytes(&[]),
    };
    run_resolved("git-lock", args, &options).into_output()
}

#[allow(dead_code)]
pub fn run_git_lock(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> String {
    let output = run_git_lock_output(dir, args, envs, input);
    if !output.status.success() {
        panic!(
            "git-lock {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}
