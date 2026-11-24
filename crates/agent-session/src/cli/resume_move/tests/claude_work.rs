use std::io::Write;
use std::{fs, path::Path};

use pretty_assertions::assert_eq;
use serde_json::json;

use crate::cli::resume_move::claude;
use crate::cli::resume_move::{History, HistoryRole, Refusal, sha256};

const THREAD: &str = "00000000-0000-4000-8000-000000000014";

#[test]
fn claude_import_preserves_exact_transcript_and_uses_destination_project() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let account = root.join("account");
    let cwd = root.join("mapped-workspace");
    fs::create_dir(&account).unwrap();
    fs::create_dir(&cwd).unwrap();
    let source_cwd = root.join("other-host").join("source-workspace");
    let path = root.join(format!("{THREAD}.jsonl"));
    let line = json!({
        "type": "user",
        "sessionId": THREAD,
        "cwd": source_cwd,
        "message": {"role": "user", "content": "synthetic conversation"},
    });
    let original = format!("{line}\n").into_bytes();
    fs::write(&path, &original).unwrap();

    let mut input = fs::File::open(&path).unwrap();
    assert_eq!(
        claude::transcript_cwd_digest(&mut input, THREAD).unwrap(),
        sha256(source_cwd.as_os_str().as_encoded_bytes())
    );
    assert!(!source_cwd.exists());

    let target = claude::Destination::observe(&account, &cwd, THREAD).unwrap();
    let metadata = History {
        role: HistoryRole::ClaudeTranscript,
        relative_path: format!("{THREAD}.jsonl"),
        bytes: original.len() as u64,
        sha256: sha256(&original),
    };
    assert!(!account.join("projects").exists());
    claude::install_transcript(&target, &path, &metadata).unwrap();
    let installed = claude::project_path(&account, &cwd, THREAD).unwrap();
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_eq!(
        claude::install_transcript(&target, &path, &metadata)
            .unwrap_err()
            .code(),
        Refusal::DestinationWriterPresent.code()
    );
}

#[test]
fn claude_transcript_refuses_external_history_dependencies() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join(format!("{THREAD}.jsonl"));
    let sidechain = json!({"sessionId":THREAD, "cwd":"/source", "isSidechain":true});
    writeln!(fs::File::create(&source).unwrap(), "{sidechain}").unwrap();
    assert_eq!(
        claude::transcript_cwd_digest(&mut fs::File::open(&source).unwrap(), THREAD)
            .unwrap_err()
            .code(),
        Refusal::HistoryUnsupported.code()
    );

    let siblings = root.path().join(THREAD).join("tool-results");
    fs::create_dir_all(&siblings).unwrap();
    fs::write(siblings.join("spill.txt"), "synthetic external content").unwrap();
    assert_eq!(
        claude::check_source_dependencies(root.path(), &source, THREAD)
            .unwrap_err()
            .code(),
        Refusal::HistoryUnsupported.code()
    );
    assert!(claude::project_path(root.path(), Path::new("relative"), THREAD).is_err());
}
