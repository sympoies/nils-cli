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
    claude::check_transcript_cwds(
        &mut input,
        THREAD,
        &sha256(source_cwd.as_os_str().as_encoded_bytes()),
    )
    .unwrap();
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
        claude::check_transcript_cwds(&mut fs::File::open(&source).unwrap(), THREAD, "unused")
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

fn transcript(root: &Path, cwds: &[&str]) -> fs::File {
    let path = root.join(format!("{THREAD}.jsonl"));
    let mut file = fs::File::create(&path).unwrap();
    for cwd in cwds {
        let row = json!({"type":"user", "sessionId":THREAD, "cwd":cwd});
        writeln!(file, "{row}").unwrap();
    }
    fs::File::open(path).unwrap()
}

#[test]
fn transcript_accepts_every_cwd_under_the_planned_checkout_root() {
    let root = tempfile::tempdir().unwrap();
    let checkout = sha256(b"/host-a/work/project");
    let mut file = transcript(
        root.path(),
        &[
            "/host-a/work/project",
            "/host-a/work/project/crates/app",
            "/host-a/work/project",
        ],
    );
    claude::check_transcript_cwds(&mut file, THREAD, &checkout).unwrap();
}

#[test]
fn transcript_refuses_a_cwd_outside_the_planned_checkout_root_with_a_specific_code() {
    let root = tempfile::tempdir().unwrap();
    let checkout = sha256(b"/host-a/work/project");
    for outside in [
        "/host-a/work/project-sibling",
        "/host-a/work",
        "/host-a/elsewhere",
        "/host-a/work/project/../other",
        "/host-a/work/project/./app",
        "/host-a/work/project/",
    ] {
        let mut file = transcript(root.path(), &["/host-a/work/project", outside]);
        assert_eq!(
            claude::check_transcript_cwds(&mut file, THREAD, &checkout)
                .unwrap_err()
                .code(),
            Refusal::TranscriptCwdOutsideCheckout.code(),
            "{outside}"
        );
    }
    // A first row outside the root is refused the same way.
    let mut file = transcript(root.path(), &["/host-a/elsewhere"]);
    assert_eq!(
        claude::check_transcript_cwds(&mut file, THREAD, &checkout)
            .unwrap_err()
            .code(),
        Refusal::TranscriptCwdOutsideCheckout.code()
    );
}

#[test]
fn claude_destination_refuses_a_transcript_in_another_project_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let account = root.join("account");
    let cwd = root.join("mapped-workspace");
    let other = account.join("projects").join("unrelated-project");
    fs::create_dir_all(&other).unwrap();
    fs::create_dir(&cwd).unwrap();
    fs::write(other.join(format!("{THREAD}.jsonl")), "{}\n").unwrap();
    assert_eq!(
        claude::Destination::observe(&account, &cwd, THREAD)
            .err()
            .unwrap()
            .code(),
        Refusal::DestinationWriterPresent.code()
    );
}
