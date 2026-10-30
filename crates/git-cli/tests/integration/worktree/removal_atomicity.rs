//! A failed removal proof or destructive child must leave the target intact,
//! registered and retryable, and the error must say whether it mutated files.
use super::{RemovalFixture, git, parse_json};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn real_git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .expect("git on PATH")
}

/// Install a `git` shim that passes through except for one injected fault.
fn inject(fixture: &RemovalFixture, fault: &str) {
    fixture.probes.write_exe(
        "git",
        &format!(
            "#!/bin/sh\nfor last; do :; done\ncase \"$*\" in\n{fault}\nesac\nexec {} \"$@\"\n",
            real_git().display()
        ),
    );
}

fn clear_fault(fixture: &RemovalFixture) {
    fs::remove_file(fixture.probes.path().join("git")).unwrap();
}

struct Manifest {
    index: String,
    status: String,
    diff: String,
    files: Vec<(PathBuf, Vec<u8>)>,
}

fn manifest(target: &Path) -> Manifest {
    let tracked = git(
        target,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    );
    let mut files: Vec<_> = tracked
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| (PathBuf::from(path), fs::read(target.join(path)).unwrap()))
        .collect();
    files.sort();
    Manifest {
        index: git(target, &["ls-files", "-s"]),
        status: git(
            target,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        ),
        diff: git(target, &["diff", "--binary", "HEAD"]),
        files,
    }
}

fn registered(fixture: &RemovalFixture) -> bool {
    git(fixture.repo.path(), &["worktree", "list", "--porcelain"]).contains(&fixture.target)
}

fn dirty(target: &Path) {
    fs::write(target.join("README.md"), b"changed\0tracked\n").unwrap();
    fs::write(target.join("new.bin"), b"new\0untracked\xff").unwrap();
}

fn retried_ok(fixture: &RemovalFixture) {
    clear_fault(fixture);
    let retry = fixture.remove("safe");
    assert_eq!(retry.code, 0, "{}", retry.stdout_text());
    assert!(!Path::new(&fixture.target).exists());
}

fn error_of(fixture: &RemovalFixture) -> Value {
    let result = fixture.remove("safe");
    assert_ne!(result.code, 0, "{}", result.stdout_text());
    parse_json(&result)["error"].clone()
}

#[test]
fn proof_failure_reports_phase_without_mutation_and_retries() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    dirty(target);
    let before = manifest(target);
    inject(
        &fixture,
        "*\"status --porcelain\"*) echo 'fatal: injected' >&2; exit 128 ;;",
    );
    let error = error_of(&fixture);
    assert_eq!(error["code"], "removal-backup-failed");
    assert_eq!(error["details"]["phase"], "backup");
    assert_eq!(error["details"]["worktree_mutated"], false);
    assert!(registered(&fixture));
    assert_eq!(manifest(target).files, before.files);
    assert_eq!(manifest(target).index, before.index);
    retried_ok(&fixture);
}

#[test]
fn destructive_timeout_restores_dirty_bytes_and_stays_retryable() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    dirty(target);
    let before = manifest(target);
    // The real incident: the destructive child deletes content, then outlives
    // the 15s bounded runner and is killed mid-removal.
    inject(
        &fixture,
        "*\"worktree remove\"*) find \"$last\" -mindepth 1 -maxdepth 1 ! -name .git ! -type d -delete; sleep 60 ;;",
    );
    let error = error_of(&fixture);
    assert!(registered(&fixture));
    assert_eq!(manifest(target).files, before.files);
    assert_eq!(manifest(target).index, before.index);
    assert_eq!(manifest(target).status, before.status);
    assert_eq!(manifest(target).diff, before.diff);
    assert_eq!(error["code"], "removal-remove-incomplete");
    assert_eq!(error["details"]["phase"], "remove");
    assert_eq!(error["details"]["worktree_mutated"], true);
    assert_eq!(error["details"]["recovery"], "restored");
    retried_ok(&fixture);
}

#[test]
fn destructive_failure_restores_clean_tree_and_git_pointer() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    let before = manifest(target);
    inject(
        &fixture,
        "*\"worktree remove\"*) find \"$last\" -mindepth 1 -maxdepth 1 ! -type d -delete; exit 1 ;;",
    );
    let error = error_of(&fixture);
    assert!(registered(&fixture));
    assert!(target.join("README.md").is_file() && target.join(".git").is_file());
    assert_eq!(manifest(target).files, before.files);
    assert_eq!(manifest(target).index, before.index);
    assert!(manifest(target).status.is_empty());
    assert_eq!(error["code"], "removal-remove-incomplete");
    assert_eq!(error["details"]["phase"], "remove");
    assert_eq!(error["details"]["recovery"], "restored");
    retried_ok(&fixture);
}
