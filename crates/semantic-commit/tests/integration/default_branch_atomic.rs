use super::*;
use pretty_assertions::{assert_eq, assert_ne};
use std::time::{Duration, Instant};

#[test]
fn competing_writer_after_head_check_keeps_its_tip_and_no_receipt() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    let old = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let branch_log_before = git_trim(
        repo.path(),
        &["reflog", "show", "--format=%H", "refs/heads/main"],
    );
    let head_log_before = git_trim(repo.path(), &["reflog", "show", "--format=%H", "HEAD"]);
    let tree = git_trim(repo.path(), &["rev-parse", "HEAD^{tree}"]);
    let competing = git_trim(
        repo.path(),
        &[
            "commit-tree",
            &tree,
            "-p",
            &old,
            "-m",
            "test: competing writer",
        ],
    );
    stage_change(repo.path());
    let scratch = tempfile::tempdir().expect("barrier directory");
    let receipt = scratch.path().join("receipt.json");
    common::write_executable(
        scratch.path(),
        "git",
        r#"#!/bin/sh
for arg in "$@"; do
    case "$arg" in
        commit|commit-tree)
            touch "$BARRIER_DIR/ready"
            count=0
            until test -f "$BARRIER_DIR/release"; do
                count=$((count + 1))
                test "$count" -lt 1000 || exit 97
                sleep 0.01
            done
            break
            ;;
    esac
done
exec "$REAL_GIT" "$@"
"#,
    );
    let real_git = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .expect("git executable");
    let path = std::env::join_paths(std::iter::once(scratch.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").expect("PATH")),
    ))
    .expect("wrapper PATH");
    let output = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            run_default_branch_with_env(
                repo.path(),
                "test: install expected parent",
                &old,
                Some(&receipt),
                &[],
                &[
                    ("PATH", path.to_str().expect("PATH text")),
                    ("REAL_GIT", real_git.to_str().expect("git path")),
                    (
                        "BARRIER_DIR",
                        scratch.path().to_str().expect("barrier path"),
                    ),
                ],
            )
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !scratch.path().join("ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = scratch.path().join("ready").exists();
        if ready {
            common::git(
                repo.path(),
                &["update-ref", "refs/heads/main", &competing, &old],
            );
        }
        fs::write(scratch.path().join("release"), "release").expect("release barrier");
        let output = worker.join().expect("command thread");
        assert!(
            ready,
            "commit barrier was not reached: {}",
            text(&output.stderr)
        );
        output
    });
    assert_ne!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert!(!receipt.exists());
    assert_eq!(
        git_trim(repo.path(), &["rev-parse", "HEAD"]),
        competing,
        "losing command must preserve the competing branch tip"
    );
    assert_eq!(
        git_trim(
            repo.path(),
            &["rev-list", "--count", &format!("{old}..HEAD")]
        ),
        "1"
    );
    assert_eq!(
        git_trim(
            repo.path(),
            &["reflog", "show", "--format=%H", "refs/heads/main"]
        ),
        format!("{competing}\n{branch_log_before}"),
        "only the competing update may appear in branch history"
    );
    assert_eq!(
        git_trim(repo.path(), &["reflog", "show", "--format=%H", "HEAD"]),
        format!("{competing}\n{head_log_before}"),
        "only the competing update may appear in HEAD history"
    );
}

#[test]
fn native_hooks_and_message_options_are_preserved() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("hook directory");
    let hooks = scratch.path().join("hooks");
    fs::create_dir(&hooks).expect("hooks directory");
    common::git(
        repo.path(),
        &[
            "config",
            "core.hooksPath",
            hooks.to_str().expect("hooks path"),
        ],
    );
    common::write_executable(
        &hooks,
        "pre-commit",
        r#"#!/bin/sh
set -eu
test -f "$GIT_INDEX_FILE"
test "$(git symbolic-ref --short HEAD)" = main
printf 'hook change\n' > change.txt
git add change.txt
printf 'pre\n' >> "$HOOK_LOG"
"#,
    );
    common::write_executable(
        &hooks,
        "prepare-commit-msg",
        r#"#!/bin/sh
set -eu
test "$2" = message
printf 'prepare\n' >> "$HOOK_LOG"
printf '\nHook-Trailer: prepared\n' >> "$1"
"#,
    );
    common::write_executable(
        &hooks,
        "commit-msg",
        r#"#!/bin/sh
set -eu
grep -q 'Hook-Trailer: prepared' "$1"
grep -q 'Signed-off-by: Test User <test@example.com>' "$1"
grep -q 'Refs: fixture' "$1"
printf 'message\n' >> "$HOOK_LOG"
"#,
    );
    common::write_executable(
        &hooks,
        "post-commit",
        r#"#!/bin/sh
printf 'post\n' >> "$HOOK_LOG"
"#,
    );
    let log = scratch.path().join("hooks.log");
    let receipt = scratch.path().join("receipt.json");
    let output = run_default_branch_with_env(
        repo.path(),
        "test: preserve native commit hooks",
        &head,
        Some(&receipt),
        &["--signoff", "--trailer", "Refs: fixture"],
        &[("HOOK_LOG", log.to_str().expect("log path"))],
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        fs::read_to_string(&log).expect("hook log"),
        "pre\nprepare\nmessage\npost\n"
    );
    let parsed = read_strict(&receipt).expect("receipt");
    assert_eq!(parsed.data.parent_sha, head);
    for reference in ["HEAD", "refs/heads/main"] {
        assert_eq!(
            git_trim(repo.path(), &["reflog", "show", "--format=%H", reference]),
            format!("{}\n{head}", parsed.data.new_head)
        );
        assert_eq!(
            git_trim(
                repo.path(),
                &["reflog", "show", "-1", "--format=%gs", reference]
            ),
            "commit: test: preserve native commit hooks"
        );
    }
    assert_eq!(
        git_trim(repo.path(), &["show", "HEAD:change.txt"]),
        "hook change"
    );
    assert!(
        git_trim(repo.path(), &["log", "-1", "--format=%B"]).contains("Hook-Trailer: prepared")
    );
}

#[test]
fn vetoing_commit_hooks_leave_branch_and_receipt_unchanged() {
    for hook in [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "reference-transaction",
    ] {
        let repo = init_repo_with_head();
        let _signing = configure_ssh_signing(repo.path());
        stage_change(repo.path());
        let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
        let scratch = tempfile::tempdir().expect("receipt directory");
        let receipt = scratch.path().join("receipt.json");
        common::write_executable(
            repo.path(),
            &format!(".git/hooks/{hook}"),
            "#!/bin/sh\nexit 1\n",
        );
        let output = run_default_branch(
            repo.path(),
            "test: honor hook refusal",
            &head,
            Some(&receipt),
            &[],
        );
        assert_ne!(output.status.code(), Some(0), "{hook}");
        assert_eq!(
            git_trim(repo.path(), &["rev-parse", "HEAD"]),
            head,
            "{hook}"
        );
        assert!(!receipt.exists(), "{hook}");
    }
}

#[test]
fn receipt_failure_preserves_installed_commit_and_does_not_overwrite() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("receipt directory");
    let receipt = scratch.path().join("receipt.json");
    common::write_executable(
        repo.path(),
        ".git/hooks/post-commit",
        "#!/bin/sh\nprintf occupied > \"$RECEIPT_PATH\"\n",
    );
    let output = run_default_branch_with_env(
        repo.path(),
        "test: retain installed identity",
        &head,
        Some(&receipt),
        &[],
        &[("RECEIPT_PATH", receipt.to_str().expect("receipt path"))],
    );
    let installed = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    assert_ne!(installed, head);
    assert_ne!(output.status.code(), Some(0));
    assert!(text(&output.stderr).contains(&format!("commit {installed} was created")));
    assert!(text(&output.stderr).contains("do not replay"));
    assert_eq!(
        fs::read_to_string(receipt).expect("occupied receipt"),
        "occupied"
    );
    assert_eq!(
        git_trim(
            repo.path(),
            &["rev-list", "--count", &format!("{head}..HEAD")]
        ),
        "1"
    );
}

#[test]
fn later_head_move_reports_installed_identity_instead_of_current_head() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("recovery directory");
    let receipt = scratch.path().join("receipt.json");
    let identity = scratch.path().join("installed");
    let later_identity = scratch.path().join("later");
    common::write_executable(
        repo.path(),
        ".git/hooks/post-commit",
        r#"#!/bin/sh
set -eu
installed=$(git rev-parse HEAD)
printf '%s' "$installed" > "$INSTALLED_ID"
tree=$(git rev-parse 'HEAD^{tree}')
later=$(git -c commit.gpgsign=false commit-tree "$tree" -p "$installed" -m 'test: later writer')
git update-ref refs/heads/main "$later" "$installed"
printf '%s' "$later" > "$LATER_ID"
"#,
    );
    let output = run_default_branch_with_env(
        repo.path(),
        "test: preserve recovery identity",
        &head,
        Some(&receipt),
        &[],
        &[
            ("INSTALLED_ID", identity.to_str().expect("identity path")),
            (
                "LATER_ID",
                later_identity.to_str().expect("later identity path"),
            ),
        ],
    );
    let installed = fs::read_to_string(identity).expect("installed identity");
    assert_ne!(output.status.code(), Some(0));
    let later = fs::read_to_string(later_identity).expect("later identity");
    assert_eq!(git_trim(repo.path(), &["rev-parse", "HEAD"]), later);
    assert_eq!(
        git_trim(repo.path(), &["rev-parse", &format!("{later}^1")]),
        installed
    );
    assert!(text(&output.stderr).contains(&format!("commit {installed} was created")));
    assert!(!receipt.exists());
}

#[test]
fn verbatim_cleanup_rejects_an_empty_hook_message_before_installation() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    common::git(repo.path(), &["config", "commit.cleanup", "verbatim"]);
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("receipt directory");
    let receipt = scratch.path().join("receipt.json");
    common::write_executable(
        repo.path(),
        ".git/hooks/commit-msg",
        r#"#!/bin/sh
: > "$1"
"#,
    );
    let output = run_default_branch(
        repo.path(),
        "test: reject empty hook message",
        &head,
        Some(&receipt),
        &[],
    );
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(git_trim(repo.path(), &["rev-parse", "HEAD"]), head);
    assert!(!receipt.exists());
}

#[test]
fn busy_index_refuses_before_pre_commit_hook_runs() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("receipt directory");
    let receipt = scratch.path().join("receipt.json");
    let marker = scratch.path().join("hook-ran");
    common::write_executable(
        repo.path(),
        ".git/hooks/pre-commit",
        r#"#!/bin/sh
printf ran > "$HOOK_MARKER"
"#,
    );
    fs::write(repo.path().join(".git/index.lock"), "busy").expect("index lock");
    let output = run_default_branch_with_env(
        repo.path(),
        "test: refuse busy index before hooks",
        &head,
        Some(&receipt),
        &[],
        &[("HOOK_MARKER", marker.to_str().expect("marker path"))],
    );
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(git_trim(repo.path(), &["rev-parse", "HEAD"]), head);
    assert!(!receipt.exists());
    assert!(
        !marker.exists(),
        "pre-commit must not run while the index is busy"
    );
}

#[test]
fn verbatim_message_file_bytes_reach_hooks_and_commit_unchanged() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    common::git(repo.path(), &["config", "commit.cleanup", "verbatim"]);
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("message directory");
    let receipt = scratch.path().join("receipt.json");
    let source = scratch.path().join("message.txt");
    let message = "test: preserve verbatim message bytes";
    fs::write(&source, message).expect("source message");
    common::write_executable(
        repo.path(),
        ".git/hooks/commit-msg",
        r#"#!/bin/sh
cmp "$1" "$SOURCE_MESSAGE"
"#,
    );
    let output = common::run_semantic_commit_output(
        repo.path(),
        &[
            "default-branch",
            "--expect-head",
            &head,
            "--message-file",
            source.to_str().expect("message path"),
            "--receipt-out",
            receipt.to_str().expect("receipt path"),
        ],
        &[("SOURCE_MESSAGE", source.to_str().expect("message path"))],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let object = common::git_output(repo.path(), &["cat-file", "commit", "HEAD"]);
    let body = text(&object.stdout)
        .split_once("\n\n")
        .expect("commit body")
        .1;
    assert_eq!(body, message);
    read_strict(&receipt).expect("receipt");
}

#[test]
fn invalid_cleanup_refuses_before_commit_hooks_run() {
    let repo = init_repo_with_head();
    let _signing = configure_ssh_signing(repo.path());
    stage_change(repo.path());
    common::git(repo.path(), &["config", "commit.cleanup", "invalid-mode"]);
    let head = git_trim(repo.path(), &["rev-parse", "HEAD"]);
    let scratch = tempfile::tempdir().expect("receipt directory");
    let receipt = scratch.path().join("receipt.json");
    let marker = scratch.path().join("hook-ran");
    common::write_executable(
        repo.path(),
        ".git/hooks/pre-commit",
        r#"#!/bin/sh
printf ran > "$HOOK_MARKER"
"#,
    );
    let output = run_default_branch_with_env(
        repo.path(),
        "test: reject invalid cleanup before hooks",
        &head,
        Some(&receipt),
        &[],
        &[("HOOK_MARKER", marker.to_str().expect("marker path"))],
    );
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(git_trim(repo.path(), &["rev-parse", "HEAD"]), head);
    assert!(!receipt.exists());
    assert!(
        !marker.exists(),
        "invalid cleanup must refuse before pre-commit"
    );
}
