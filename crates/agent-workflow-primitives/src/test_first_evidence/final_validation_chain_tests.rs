use super::tests::{fixture, resolve};
use super::*;
use pretty_assertions::assert_eq;

fn record_final(dir: &Path, command: &str, status: &str) {
    assert_eq!(
        run_with_args([
            "test-first-evidence",
            "record-final",
            "--out",
            dir.to_str().unwrap(),
            "--command",
            command,
            "--scope",
            "focused",
            "--status",
            status,
        ]),
        0
    );
}

fn chain() -> (tempfile::TempDir, Value) {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    assert_eq!(
        resolve(
            dir.path(),
            "canonical",
            "1",
            "covered target",
            "investigated failure"
        ),
        0
    );
    let original: Value =
        serde_json::from_slice(&fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap()).unwrap();
    let resolution = original["final_validations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["command"] == "cargo test affected")
        .unwrap()["resolution"]
        .clone();
    record_final(dir.path(), "canonical", "fail");
    record_final(dir.path(), "canonical-next", "pass");
    assert_eq!(
        run_with_args([
            "test-first-evidence",
            "resolve-final",
            "--out",
            dir.path().to_str().unwrap(),
            "--command",
            "canonical",
            "--scope",
            "focused",
            "--attempt",
            "2",
            "--passing-command",
            "canonical-next",
            "--passing-scope",
            "focused",
            "--passing-attempt",
            "1",
            "--reason",
            "canonical covers the same target",
            "--investigation",
            "investigated later failure",
        ]),
        0
    );
    (dir, resolution)
}

#[test]
fn resolved_chain_preserves_history_and_admits_verify() {
    let (dir, original_resolution) = chain();
    let path = dir.path().join(RECORD_FILE_NAME);
    let before = fs::read(&path).unwrap();
    assert_eq!(
        run_with_args([
            "test-first-evidence",
            "verify",
            "--out",
            dir.path().to_str().unwrap(),
        ]),
        0
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "verify must not flatten history"
    );
    let record: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(record["final_validations"].as_array().unwrap().len(), 4);
    let attempts = record["final_validations"].as_array().unwrap();
    let first = attempts
        .iter()
        .find(|v| v["command"] == "cargo test affected")
        .unwrap();
    assert_eq!(first["resolution"], original_resolution);
    assert_eq!(first["status"], "fail");
    let later = attempts
        .iter()
        .find(|v| v["command"] == "canonical" && v["attempt"] == 2)
        .unwrap();
    assert_eq!(later["status"], "fail");
}

#[test]
fn resolved_chain_rejects_an_unresolved_latest_failure_despite_an_unrelated_pass() {
    let (dir, _) = chain();
    record_final(dir.path(), "canonical-next", "fail");
    record_final(dir.path(), "unrelated", "pass");
    let result = verify_dir(dir.path()).unwrap();
    assert!(!result.complete);
    assert!(
        result
            .missing
            .iter()
            .any(|s| s == "failed_final_validation")
    );
}

#[test]
fn resolved_chain_rejects_missing_or_incorrect_passing_references() {
    for (field, value) in [
        ("passing_command", json!("absent")),
        ("passing_attempt", json!(99)),
        ("passing_scope", json!("full")),
    ] {
        let (dir, _) = chain();
        let path = dir.path().join(RECORD_FILE_NAME);
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let failed = record["final_validations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|v| v["command"] == "canonical" && v["attempt"] == 2)
            .unwrap();
        failed["resolution"][field] = value;
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let result = verify_dir(dir.path()).unwrap();
        assert!(!result.complete, "{field}");
        assert!(
            result
                .missing
                .iter()
                .any(|s| s == "invalid_final_resolution")
        );
    }
}

#[test]
fn resolved_chain_rejects_self_and_multi_command_cycles() {
    for target in ["canonical-next", "canonical"] {
        let (dir, _) = chain();
        record_final(dir.path(), "canonical-next", "fail");
        let path = dir.path().join(RECORD_FILE_NAME);
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let failed = record["final_validations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|v| v["command"] == "canonical-next" && v["attempt"] == 2)
            .unwrap();
        failed["resolution"] = json!({
            "passing_command": target, "passing_scope": "focused", "passing_attempt": 1,
            "reason": "cyclic historical reference", "investigation": ["cycle fixture"],
        });
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let result = verify_dir(dir.path()).unwrap();
        assert!(!result.complete, "{target}");
        assert!(
            result
                .missing
                .iter()
                .any(|s| s == "failed_final_validation")
        );
    }
}
