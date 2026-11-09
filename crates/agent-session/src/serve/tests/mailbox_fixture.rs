// Synthetic native notice rows use the same physical response/client event pair.
use super::*;
use std::io::Write;

pub(super) fn path(root: &Path, id: &str) -> PathBuf {
    root.join("codex-home/sessions").join(format!("{id}.jsonl"))
}

pub(super) fn bind(context: &CliContext, id: &str, root: &Path, provider: &str) -> SessionRecord {
    let mut record = load_session_record(context, id).unwrap();
    record.provider_resume = Some(
        serde_json::from_value(json!({"provider":"codex","session_id":provider,
        "captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":[]}))
        .unwrap(),
    );
    record.runtime.as_mut().unwrap().extra.insert(
        "agent_profile_provider_config_dir".into(),
        json!(root.join("codex-home")),
    );
    crate::write_session_record(context, &record).unwrap();
    let transcript = path(root, id);
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(transcript, format!("{}\n", json!({"type":"session_meta","timestamp":"2026-01-01T00:00:00Z",
        "payload":{"id":provider,"cwd":record.cwd,"source":"cli","timestamp":"2026-01-01T00:00:00Z"}}))).unwrap();
    record
}

pub(super) fn append(path: &Path, text: &str, client: &str, turn: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file,"{}",json!({"type":"response_item","timestamp":"2030-01-01T00:00:01Z","payload":{"type":"message","role":"user",
        "content":[{"type":"input_text","text":text}],
        "internal_chat_message_metadata_passthrough":{"turn_id":turn,"content_item_kinds":["user.text"]}}})).unwrap();
    writeln!(file,"{}",json!({"type":"event_msg","timestamp":"2030-01-01T00:00:01Z","payload":{"type":"user_message","client_id":client,"message":text}})).unwrap();
}
