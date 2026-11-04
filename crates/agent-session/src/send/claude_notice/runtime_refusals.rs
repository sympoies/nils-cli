//! Private filesystem refusals retain a stable reason without path disclosure.
use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::{PermissionsExt, symlink};

fn reason<T>(result: Result<T, CliError>, expected: &str) {
    let error = result.err().expect("must refuse");
    assert_eq!(error.code(), "notice-native-unavailable");
    assert!(error.message().contains(expected));
    assert_eq!(error.details().unwrap()["reason"], expected);
    assert_eq!(error.details().unwrap().as_object().unwrap().len(), 1);
}

#[test]
fn private_directory_refusals_identify_creation_and_validation() {
    let temp = tempfile::tempdir().unwrap();
    reason(
        private_dir(&temp.path().join("missing/child")),
        "private-directory-create",
    );
    let file = temp.path().join("file");
    fs::write(&file, "synthetic").unwrap();
    reason(private_dir(&file), "private-directory-invalid");
    let directory = temp.path().join("directory");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    reason(private_dir(&directory), "private-directory-invalid");
}

#[test]
fn private_file_refusals_identify_lookup_open_and_validation() {
    let temp = tempfile::tempdir().unwrap();
    reason(
        private_read(&temp.path().join("missing"), 32),
        "private-file-inspect",
    );
    let file = temp.path().join("file");
    fs::write(&file, "synthetic").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    reason(private_read(&file, 2), "private-file-invalid");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    reason(private_read(&file, 32), "private-file-invalid");
    let link = temp.path().join("link");
    symlink(&file, &link).unwrap();
    reason(private_read(&link, 32), "private-file-open");
    reason(write_new(&file, b"synthetic"), "private-file-create");
}

#[test]
fn module_directory_refuses_unsafe_incarnations_with_a_fixed_reason() {
    for incarnation in ["..", "a/b", "x".repeat(129).as_str()] {
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"fixture", "agent":"claude",
            "mode":"interactive", "cwd":"/workspace/project", "tmux_session":"fixture",
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
            "runtime":{"kind":"tmux", "tmux_session":"fixture", "generation":1,
                "started_at":"2026-01-01T00:00:00Z", "launch_id":incarnation}
        }))
        .unwrap();
        reason(
            module_dir(Path::new("/workspace/state"), &record),
            "incarnation-invalid",
        );
    }
}
