//! A failed removal proof or destructive child must leave the target intact,
//! registered and retryable, and the error must say whether it mutated files.
use super::{RemovalFixture, git, parse_json};
use nils_test_support::cmd::run_with;
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
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
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
        .map(|path| (PathBuf::from(path), fs::read(target.join(path)).ok()))
        .collect();
    files.sort();
    Manifest {
        index: git(target, &["ls-files", "-s"]),
        status: git(
            target,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        ),
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
    error_of_with(fixture, &[])
}

fn error_of_with(fixture: &RemovalFixture, extra: &[&str]) -> Value {
    let mut args = vec!["worktree", "remove", "safe"];
    args.extend(extra);
    args.extend(["--format", "json"]);
    let result = run_with(
        &fixture.harness.git_cli_bin(),
        &args,
        &fixture.options(fixture.repo.path()),
    );
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
    assert_eq!(error["details"]["worktree_mutation"], "none");
    assert!(registered(&fixture));
    assert_eq!(manifest(target).files, before.files);
    assert_eq!(manifest(target).index, before.index);
    retried_ok(&fixture);
}

#[test]
fn destructive_timeout_restores_only_missing_snapshot_entries() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    fs::write(target.join("gone.txt"), "gone\n").unwrap();
    fs::write(target.join("kept.txt"), "kept\n").unwrap();
    fixture.commit();
    dirty(target);
    fs::remove_file(target.join("gone.txt")).unwrap();
    let before = manifest(target);
    // The real incident: the destructive child deletes content, then outlives
    // the 15s bounded runner and is killed mid-removal. An entry rewritten
    // after the snapshot must survive the recovery untouched.
    inject(
        &fixture,
        "*\"worktree remove\"*) find \"$last\" -mindepth 1 -maxdepth 1 ! -name .git ! -name README.md ! -type d -delete; echo 'edited after snapshot' > \"$last/README.md\"; sleep 60 ;;",
    );
    let error = error_of(&fixture);
    assert!(registered(&fixture));
    let after = manifest(target);
    assert_eq!(after.index, before.index);
    assert!(!target.join("gone.txt").exists());
    let mut expected = before.files;
    for (path, bytes) in &mut expected {
        if path == Path::new("README.md") {
            *bytes = Some(b"edited after snapshot\n".to_vec());
        }
    }
    assert_eq!(after.files, expected);
    assert_eq!(error["code"], "removal-remove-incomplete");
    assert_eq!(error["details"]["phase"], "remove");
    assert_eq!(error["details"]["worktree_mutation"], "started");
    assert_eq!(error["details"]["recovery"], "restored");
    assert!(error["details"]["backup_ref"].is_string());
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
    assert_eq!(error["details"]["worktree_mutation"], "started");
    assert_eq!(error["details"]["recovery"], "restored");
    retried_ok(&fixture);
}

#[test]
fn deleted_untracked_omission_is_reported_partial_and_never_restored() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    git(
        fixture.repo.path(),
        &["config", "worktree.backupMaxBytes", "8"],
    );
    fs::write(target.join("large.bin"), b"0123456789abcdef").unwrap();
    inject(
        &fixture,
        "*\"worktree remove\"*) unlink \"$last/large.bin\"; exit 1 ;;",
    );
    let error = error_of_with(&fixture, &["--acknowledge-backup-omissions"]);
    assert!(!target.join("large.bin").exists());
    assert_eq!(error["code"], "removal-remove-incomplete");
    assert_eq!(error["details"]["recovery"], "partial");
    assert_eq!(error["details"]["worktree_mutation"], "started");
    assert_eq!(error["details"]["backup_omissions"][0]["path"], "large.bin");
}

#[test]
fn replaced_parent_symlink_is_refused_without_writing_outside() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    fs::create_dir(target.join("sub")).unwrap();
    fs::write(target.join("sub/a.txt"), "tracked\n").unwrap();
    fixture.commit();
    let outside = tempfile::tempdir().unwrap();
    inject(
        &fixture,
        &format!(
            "*\"worktree remove\"*) unlink \"$last/sub/a.txt\"; rmdir \"$last/sub\"; ln -s {} \"$last/sub\"; exit 1 ;;",
            outside.path().display()
        ),
    );
    let error = error_of(&fixture);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(
        fs::symlink_metadata(target.join("sub"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(error["code"], "removal-remove-incomplete");
    assert_eq!(error["details"]["recovery"], "failed");
    assert_eq!(error["details"]["worktree_mutation"], "started");
    assert!(registered(&fixture));
}
