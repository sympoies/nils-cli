mod common;

use std::fs;

#[test]
fn reports_missing_git_in_path() {
    let temp = tempfile::TempDir::new().unwrap();

    let stub = tempfile::TempDir::new().unwrap();
    let git_path = stub.path().join("git");
    fs::write(&git_path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&git_path).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&git_path, perms).unwrap();
    }

    let path_env = stub.path().to_string_lossy().to_string();
    let (code, output) =
        common::run_git_summary_allow_fail(temp.path(), &["all"], &[("PATH", path_env.as_str())]);

    assert_ne!(code, 0);
    assert!(output.contains("git is required but was not found in PATH."));
}
