use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::symlink;

#[test]
fn private_test_worker_resolves_temporary_directory_aliases() {
    const CHILD_SOURCE: &str = "NILS_TEST_WORKER_ALIAS_SOURCE";
    const TEST_NAME: &str = "worktree::dirty_checkout_adoption::tests::worker_path::private_test_worker_resolves_temporary_directory_aliases";

    if let Some(source) = env::var_os(CHILD_SOURCE) {
        let resolved = private_test_worker_executable(PathBuf::from(source))
            .expect("install worker through temporary-directory alias");
        let canonical = fs::canonicalize(&resolved).expect("resolve installed worker");
        assert!(
            resolved == canonical,
            "materialized test worker must use a canonical path before trust validation"
        );
        validate_worker_file_metadata(&fs::metadata(&resolved).expect("worker metadata"))
            .expect("materialized worker must retain trusted ownership and mode");
        return;
    }

    // Isolate TMPDIR in a child: changing it in this process would race other tests.
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    let temporary = root.path().join("temporary");
    fs::create_dir(&temporary).expect("create temporary root");
    let alias = root.path().join("alias");
    symlink(&temporary, &alias).expect("create temporary-directory alias");
    let source = root.path().join("git-cli");
    fs::write(&source, b"worker fixture").expect("write worker source");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o770))
        .expect("force private worker materialization");

    let output = Command::new(env::current_exe().expect("current test executable"))
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_SOURCE, source)
        .env("TMPDIR", alias)
        .output()
        .expect("run isolated worker alias test");
    assert!(
        output.status.success(),
        "worker alias child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn worker_path_validation_rejects_symlink_and_writable_parents() {
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    let directory = root.path().join("worker");
    fs::create_dir(&directory).expect("create worker directory");
    let executable = directory.join("git-cli");
    fs::write(&executable, b"worker fixture").expect("write worker source");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o500))
        .expect("harden worker fixture");
    validate_worker_file_metadata(&fs::metadata(&executable).expect("worker metadata"))
        .expect("file itself must be trusted");

    let alias = root.path().join("alias");
    symlink(&directory, &alias).expect("create worker parent alias");
    let error = validate_worker_path_permissions(&alias.join("git-cli"))
        .expect_err("production path validation must still reject symlink parents");
    assert_eq!(
        error.to_string(),
        "dirty snapshot worker executable path is not trusted"
    );

    for mode in [0o770, 0o707] {
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode))
            .expect("make worker parent writable");
        let error = validate_worker_path_permissions(&executable)
            .expect_err("production path validation must still reject writable parents");
        assert_eq!(
            error.to_string(),
            "dirty snapshot worker executable path is not trusted"
        );
    }
}
