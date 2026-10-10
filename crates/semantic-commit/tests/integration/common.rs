use std::path::Path;
use std::process::Output;

use nils_test_support::cmd::run_resolved_in_dir_with_stdin_str;
use nils_test_support::fs::{write_executable_in_dir, write_text_in_dir};
use nils_test_support::git::init_repo_main;
#[allow(unused_imports)]
pub use nils_test_support::git::{git, git_output};

#[allow(dead_code)]
pub fn init_repo() -> tempfile::TempDir {
    init_repo_main()
}

#[allow(dead_code)]
pub fn write_file(dir: &Path, name: &str, contents: &str) {
    write_text_in_dir(dir, name, contents);
}

#[allow(dead_code)]
pub fn run_semantic_commit_output(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> Output {
    // Point forge identity and config lookups at an empty per-run directory, so
    // the developer's forge profiles cannot leak into the child process.
    // Callers may still override these through `envs`.
    let isolated = tempfile::TempDir::new().expect("isolated config dir");
    let xdg_config = isolated.path().join("xdg-config");
    let xdg_state = isolated.path().join("xdg-state");
    let mut all_envs = vec![
        ("XDG_CONFIG_HOME", xdg_config.to_str().expect("config path")),
        ("XDG_STATE_HOME", xdg_state.to_str().expect("state path")),
    ];
    all_envs.extend_from_slice(envs);
    let output = run_resolved_in_dir_with_stdin_str("semantic-commit", dir, args, &all_envs, input);
    output.into_output()
}

#[allow(dead_code)]
pub fn write_executable(dir: &Path, rel: &str, contents: &str) {
    write_executable_in_dir(dir, rel, contents);
}
