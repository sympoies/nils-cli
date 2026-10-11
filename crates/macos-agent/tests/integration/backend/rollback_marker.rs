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
