use pretty_assertions::assert_eq;

use super::*;

/// Leave only the rollback marker, naming `receipt` as the staged target, as a crash would.
fn leave_only_the_marker(root: &Path, receipt: &str) {
    fs::copy(
        root.join("receipts").join(receipt),
        root.join("receipts/pending-rollback.json"),
    )
    .expect("marker");
    assert!(!root.join("receipts/pending.json").exists());
}

fn installed_old_then_new(
    harness: &common::MacosAgentHarness,
    cwd: &Path,
    root: &Path,
) -> (Candidate, Candidate) {
    let old = candidate(cwd, "v4.6.0", '6');
    let new = candidate(cwd, "v4.8.0", '8');
    authorize_rollback(&new, &old);
    for release in [&old, &new] {
        let out = run_backend(harness, cwd, root, release, &["backend", "install"]);
        assert_eq!(out.code, 0, "{}", out.stderr_text());
    }
    (old, new)
}

fn assert_rolled_back_to_old(out: &nils_test_support::cmd::CmdOutput, root: &Path) {
    assert_eq!(out.code, 0, "{}", out.stderr_text());
    assert_eq!(out.stdout_json()["result"]["current"]["tag"], "v4.6.0");
    assert_eq!(out.stdout_json()["result"]["previous"]["tag"], "v4.8.0");
    assert!(!root.join("receipts/pending.json").exists());
    assert!(!root.join("receipts/pending-rollback.json").exists());
}

#[test]
fn review_repair_rollback_retry_after_commit_with_only_the_marker_left_succeeds() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let root = cwd.path().join("backend");
    let (_old, new) = installed_old_then_new(&harness, cwd.path(), &root);
    let committed = run_backend(&harness, cwd.path(), &root, &new, &["backend", "rollback"]);
    assert_eq!(committed.code, 0, "{}", committed.stderr_text());
    // Crash after the rollback committed, before its marker was cleared.
    leave_only_the_marker(&root, "current.json");
    let retry = run_backend(
        &harness,
        cwd.path(),
        &root,
        &new,
        &["--format", "json", "backend", "rollback"],
    );
    assert_rolled_back_to_old(&retry, &root);
}

#[test]
fn review_repair_rollback_retry_before_staging_with_only_the_marker_left_proceeds() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let root = cwd.path().join("backend");
    let (_old, new) = installed_old_then_new(&harness, cwd.path(), &root);
    // Crash after the rollback wrote its marker, before it staged the pending receipt.
    leave_only_the_marker(&root, "previous.json");
    let retry = run_backend(
        &harness,
        cwd.path(),
        &root,
        &new,
        &["--format", "json", "backend", "rollback"],
    );
    assert_rolled_back_to_old(&retry, &root);
}

#[test]
fn review_repair_install_settles_a_committed_rollback_marker() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let root = cwd.path().join("backend");
    let (_old, new) = installed_old_then_new(&harness, cwd.path(), &root);
    // A rollback onto the locked release committed but crashed before clearing its
    // marker: current is the locked release, previous is the authenticated one.
    leave_only_the_marker(&root, "current.json");
    let install = run_backend(&harness, cwd.path(), &root, &new, &["backend", "install"]);
    assert_eq!(install.code, 0, "{}", install.stderr_text());
    assert!(
        !root.join("receipts/pending-rollback.json").exists(),
        "a successful install must settle the committed rollback marker"
    );
    let preview = run_backend(
        &harness,
        cwd.path(),
        &root,
        &new,
        &["--format", "json", "backend", "rollback", "--dry-run"],
    );
    assert_eq!(preview.code, 0, "{}", preview.stderr_text());
    assert_eq!(preview.stdout_json()["result"]["current"]["tag"], "v4.6.0");
    let rollback = run_backend(
        &harness,
        cwd.path(),
        &root,
        &new,
        &["--format", "json", "backend", "rollback"],
    );
    assert_rolled_back_to_old(&rollback, &root);
}

#[test]
fn review_repair_committed_rollback_retry_verifies_the_active_target() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let root = cwd.path().join("backend");
    let (_old, new) = installed_old_then_new(&harness, cwd.path(), &root);
    let committed = run_backend(&harness, cwd.path(), &root, &new, &["backend", "rollback"]);
    assert_eq!(committed.code, 0, "{}", committed.stderr_text());
    leave_only_the_marker(&root, "current.json");
    // The committed target's stable executable changes before the retry.
    fs::write(root.join("stable/peekaboo"), "changed executable").expect("tamper stable CLI");
    let retry = run_backend(
        &harness,
        cwd.path(),
        &root,
        &new,
        &["--error-format", "json", "backend", "rollback", "--strict"],
    );
    assert_eq!(retry.code, 69, "{}", retry.stderr_text());
    assert!(
        root.join("receipts/pending-rollback.json").exists(),
        "a failed retry must keep its recovery record"
    );
}
