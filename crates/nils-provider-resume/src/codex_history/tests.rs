use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn history(path: &Path, id: &str) {
    fs::write(path, format!("{}\n", json!({"type":"session_meta", "payload":{
        "id":id, "cwd":"/workspace/project", "source":"cli", "timestamp":"2026-01-01T00:00:00Z"
    }}))).unwrap();
}

fn budget(max_entries: usize) -> CodexResumeScanBudget {
    CodexResumeScanBudget {
        visited: 0,
        max_entries,
        deadline: Instant::now() + Duration::from_secs(60),
        truncated: false,
    }
}

#[test]
fn complete_snapshot_matches_single_id_discovery_for_arbitrary_names_and_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("nested")).unwrap();
    history(&tmp.path().join("arbitrary.jsonl"), "first");
    history(&tmp.path().join("nested/anything.jsonl"), "second");
    history(&tmp.path().join("duplicate-one.jsonl"), "duplicate");
    history(&tmp.path().join("duplicate-two.jsonl"), "duplicate");
    fs::write(tmp.path().join("invalid.jsonl"), "{}\n").unwrap();
    history(&tmp.path().join("ignored.txt"), "ignored");
    let mut scan = budget(100);
    let snapshot = CodexHistorySnapshot::scan(tmp.path(), &mut scan).unwrap();
    assert_eq!(scan.visited, 7);
    for id in ["first", "second", "duplicate", "missing", "ignored"] {
        let mut matches = BTreeSet::new();
        collect_codex_provider_resume_matches(tmp.path(), 0, id, &mut matches, &mut budget(100));
        let expected = if matches.len() == 1 {
            matches.first().map(|m| m.path.as_path())
        } else {
            None
        };
        assert_eq!(snapshot.unique_path(id), expected, "{id}");
    }
}

#[test]
fn partial_or_expired_scan_never_exposes_a_seen_unique_source() {
    let tmp = tempfile::tempdir().unwrap();
    history(&tmp.path().join("first.jsonl"), "target");
    history(&tmp.path().join("second.jsonl"), "another");
    let mut short = budget(1);
    assert!(CodexHistorySnapshot::scan(tmp.path(), &mut short).is_none());
    assert_eq!(short.visited, 1);
    assert!(short.truncated);
    let mut expired = budget(100);
    expired.deadline = Instant::now() - Duration::from_secs(1);
    assert!(CodexHistorySnapshot::scan(tmp.path(), &mut expired).is_none());
    assert_eq!(expired.visited, 0);
    assert!(expired.truncated);
}

#[cfg(unix)]
#[test]
fn snapshot_ignores_symlink_files_and_directories_like_single_id_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    history(&outside.path().join("target.jsonl"), "outside");
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("linked-directory")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("target.jsonl"),
        tmp.path().join("linked-file.jsonl"),
    )
    .unwrap();
    assert!(
        CodexHistorySnapshot::scan(tmp.path(), &mut budget(100))
            .unwrap()
            .unique_path("outside")
            .is_none()
    );
}
