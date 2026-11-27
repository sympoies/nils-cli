use std::fs;
use std::path::Path;
use std::process::Command;

pub(super) fn git(dir: &Path, args: &[&str]) {
    git_with_env(dir, args, &[]);
}

pub(super) fn git_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let config = dir.join(".git/fixture-config");
    let global = if config.parent().unwrap().is_dir() {
        fs::write(&config, "[commit]\n    gpgsign = false\n").expect("write Git config");
        config.as_path()
    } else {
        Path::new("/dev/null")
    };
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS");
    for (key, value) in envs {
        command.env(key, value);
    }
    let status = command.status().expect("run git");
    assert!(status.success(), "git {args:?} failed");
}
