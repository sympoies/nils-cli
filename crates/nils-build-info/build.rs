use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn absolutize(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }

    Some(env::current_dir().ok()?.join(path))
}

fn git_dir(flag: &str) -> Option<PathBuf> {
    command_stdout("git", &["rev-parse", "--path-format=absolute", flag])
        .map(PathBuf::from)
        .or_else(|| command_stdout("git", &["rev-parse", flag]).and_then(absolutize))
}

fn watch_existing(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn watch_creation(path: &Path) {
    // Cargo treats missing paths as perpetually dirty. An existing ancestor
    // observes creation of a loose ref or tag directory without that loop.
    let mut existing = path;
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return;
        };
        existing = parent;
    }
    watch_existing(existing);
}

fn emit_git_rerun_paths() {
    let Some(worktree_git_dir) = git_dir("--git-dir") else {
        return;
    };
    watch_existing(&worktree_git_dir.join("HEAD"));
    let common_dir = git_dir("--git-common-dir").unwrap_or(worktree_git_dir);
    if let Some(reference) = command_stdout("git", &["symbolic-ref", "-q", "HEAD"]) {
        watch_creation(&common_dir.join(reference));
    }
    // Other worktrees' branch updates do not change this HEAD's description.
    watch_creation(&common_dir.join("refs/tags"));
    watch_existing(&common_dir.join("packed-refs"));
}

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");

    let describe = command_stdout("git", &["describe", "--tags", "--always", "--dirty"])
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=NILS_GIT_DESCRIBE={describe}");

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_version = command_stdout(&rustc, &["--version"])
        .map(|value| value.strip_prefix("rustc ").unwrap_or(&value).to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=NILS_RUSTC_VERSION={rustc_version}");

    emit_git_rerun_paths();
}
