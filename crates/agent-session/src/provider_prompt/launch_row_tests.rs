use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io::Write;

const NOTICE: &str =
    "<!-- agent-notice:v1 kind=session.handover source=synthetic -->\n\nSynthetic task\n";

fn append(path: &Path, row: Value) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{row}").unwrap();
}

fn wrapped() -> String {
    format!("\n\n<pasted_content id=\"abcd\">\n{NOTICE}</pasted_content id=\"abcd\">\n")
}

/// A Claude transcript whose first user row is the receipt-bound launch row,
/// followed by an assistant reply, plus the binding the launch receipt yields.
fn launched(path: &Path) -> (u64, String, String) {
    let launch = json!({"type":"user", "sessionId":"synthetic", "isSidechain":false,
        "uuid":"launch", "message":{"role":"user", "content":wrapped()}});
    fs::write(path, "").unwrap();
    append(
        path,
        json!({"type":"mode", "sessionId":"synthetic", "isSidechain":false}),
    );
    let offset = fs::metadata(path).unwrap().len();
    append(path, launch.clone());
    append(
        path,
        json!({"type":"assistant", "sessionId":"synthetic", "isSidechain":false,
        "message":{"role":"assistant", "content":[{"type":"text", "text":"ok"}]}}),
    );
    let line = format!("{launch}\n");
    (
        offset,
        NOTICE.to_string(),
        crate::send::claude_notice::sha256(native_notice::payload(line.as_bytes())),
    )
}

fn open(path: &Path, binding: &(u64, String, String)) -> ProviderLastPromptTracker {
    let source = ProviderPromptSource {
        provider: ProviderKind::Claude,
        session_id: "synthetic".to_string(),
        path: path.to_path_buf(),
    };
    let mut tracker =
        ProviderLastPromptTracker::open_source_with_validation(source, false, Some(binding))
            .unwrap();
    tracker.tail.claude_fallback_delay = Duration::ZERO;
    tracker
}

fn current(tracker: &mut ProviderLastPromptTracker) -> Option<String> {
    let ProviderLastPromptRefresh::Current(prompt) = tracker.refresh() else {
        panic!("current preview")
    };
    prompt.map(|prompt| prompt.text)
}

#[test]
fn launch_row_echoes_do_not_become_the_last_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    let binding = launched(&path);
    let echoes = [
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":wrapped()}),
        json!({"type":"queue-operation", "operation":"enqueue", "sessionId":"synthetic",
        "content":wrapped()}),
    ];
    for echo in echoes {
        // Cold recovery over an echo already on disk.
        append(&path, echo.clone());
        let mut cold = open(&path, &binding);
        assert_eq!(current(&mut cold), None, "cold: {echo}");
        // Live tail over an echo appended after the tracker opened.
        let mut live = open(&path, &binding);
        append(&path, echo.clone());
        assert_eq!(current(&mut live), None, "live: {echo}");
    }
}

#[test]
fn real_prompt_after_launch_row_wins_over_launch_row_echoes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    let binding = launched(&path);
    append(
        &path,
        json!({"type":"user", "sessionId":"synthetic", "isSidechain":false, "promptSource":"typed",
        "uuid":"real", "message":{"role":"user", "content":"real prompt"}}),
    );
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":wrapped()}),
    );
    let mut cold = open(&path, &binding);
    assert_eq!(current(&mut cold), Some("real prompt".to_string()));
    let mut live = open(&path, &binding);
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":wrapped()}),
    );
    assert_eq!(current(&mut live), Some("real prompt".to_string()));
}

#[test]
fn unbound_pasted_header_still_shows() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    let mut binding = launched(&path);
    // A receipt for a different row: the same text elsewhere is a user paste.
    binding.2 = "0".repeat(64);
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":wrapped()}),
    );
    let mut cold = open(&path, &binding);
    assert_eq!(current(&mut cold), Some(wrapped()));
}
