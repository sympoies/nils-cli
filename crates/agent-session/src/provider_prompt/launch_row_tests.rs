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
    wrap(NOTICE)
}

fn wrap(notice: &str) -> String {
    format!("\n\n<pasted_content id=\"abcd\">\n{notice}</pasted_content id=\"abcd\">\n")
}

/// A Claude transcript whose first user row is the receipt-bound launch row,
/// followed by an assistant reply, plus the binding the launch receipt yields.
fn launched(path: &Path) -> (u64, String, String) {
    launched_notice(path, NOTICE)
}

fn launched_notice(path: &Path, notice: &str) -> (u64, String, String) {
    let launch = json!({"type":"user", "sessionId":"synthetic", "isSidechain":false,
        "uuid":"launch", "message":{"role":"user", "content":wrap(notice)}});
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
        notice.to_string(),
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
fn pasted_header_under_a_wrong_hash_binding_still_shows() {
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

#[test]
fn pasted_header_without_a_binding_still_shows() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    launched(&path);
    let truncated = long_notice_echo();
    for echo in [wrapped(), truncated] {
        append(
            &path,
            json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":echo}),
        );
        let source = ProviderPromptSource {
            provider: ProviderKind::Claude,
            session_id: "synthetic".to_string(),
            path: path.clone(),
        };
        let mut tracker =
            ProviderLastPromptTracker::open_source_with_validation(source, false, None).unwrap();
        tracker.tail.claude_fallback_delay = Duration::ZERO;
        assert_eq!(current(&mut tracker), Some(echo));
    }
}

const LONG_BODY: &str = "Synthetic task with a body that is deliberately longer than the provider's two hundred character echo limit so the echoed text is cut short. It keeps going with more filler words to stay well past the limit.\n";

fn long_notice() -> String {
    format!("<!-- agent-notice:v1 kind=session.handover source=synthetic -->\n\n{LONG_BODY}")
}

/// Claude echoes the unwrapped notice with newlines flattened to spaces, cut
/// to 200 characters plus an ellipsis.
fn long_notice_echo() -> String {
    let flat = long_notice().replace('\n', " ");
    assert!(flat.chars().count() > 200);
    let cut: String = flat.chars().take(200).collect();
    format!("{cut}…")
}

#[test]
fn truncated_launch_row_echoes_do_not_become_the_last_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    let binding = launched_notice(&path, &long_notice());
    let cut = long_notice_echo();
    // Trailing whitespace before the cut can be trimmed, leaving 199 characters.
    let trimmed: String = cut.chars().take(198).collect::<String>() + "…";
    let echoes = [
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":cut}),
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":trimmed}),
        json!({"type":"queue-operation", "operation":"enqueue", "sessionId":"synthetic",
        "content":cut}),
    ];
    for echo in echoes {
        append(&path, echo.clone());
        let mut cold = open(&path, &binding);
        assert_eq!(current(&mut cold), None, "cold: {echo}");
        let mut live = open(&path, &binding);
        append(&path, echo.clone());
        assert_eq!(current(&mut live), None, "live: {echo}");
    }
}

#[test]
fn truncated_text_that_is_not_a_notice_prefix_stays_visible_under_a_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    let binding = launched_notice(&path, &long_notice());
    let other = long_notice_echo().replace("Synthetic task", "Different task");
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":other}),
    );
    let mut cold = open(&path, &binding);
    assert_eq!(current(&mut cold), Some(other));
}
