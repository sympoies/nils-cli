//! Codex 0.162.1 response-row/client-event association; no text-only authority.
use super::*;
use crate::send::claude_notice::{sha256, store};
use native_notice::{Baseline, Proof, Row};
use std::io::{BufRead, BufReader};

pub(crate) fn capture(record: &SessionRecord) -> Option<Baseline> {
    let source = ProviderPromptTail::resolve_source(record)?;
    (source.provider == ProviderKind::Codex).then_some(())?;
    Baseline::pin(source.validated_path()?.to_owned())
}

fn client(value: &Value) -> Option<&str> {
    if value["type"] != "event_msg" {
        return None;
    }
    match value["payload"]["type"].as_str()? {
        "user_message" => value["payload"]["client_id"].as_str(),
        // Codex 0.162.1 persists completed canonical items in paginated rollouts.
        "item_completed" if value["payload"]["item"]["type"] == "UserMessage" => {
            value["payload"]["item"]["client_id"].as_str()
        }
        _ => None,
    }
}

fn event_matches(value: &Value, id: &str, text: &str, turn: &str, provider: &str) -> bool {
    if client(value) != Some(id) {
        return false;
    }
    let payload = &value["payload"];
    if payload["type"] == "user_message" {
        return payload["message"].as_str() == Some(text)
            && ["images", "local_images", "file_ids", "audio", "local_audio"]
                .iter()
                .all(|key| {
                    payload
                        .get(key)
                        .is_none_or(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
                });
    }
    let content = payload["item"]["content"].as_array();
    payload["thread_id"].as_str() == Some(provider)
        && payload["turn_id"].as_str() == Some(turn)
        && content.is_some_and(|parts| {
            parts.len() == 1
                && parts[0]["type"] == "text"
                && parts[0]["text"].as_str() == Some(text)
        })
}

fn response(value: &Value, turn: &str) -> Option<Row> {
    if value["type"] != "response_item"
        || value["payload"]["type"] != "message"
        || value["payload"]["role"] != "user"
        || value
            .pointer("/payload/internal_chat_message_metadata_passthrough/turn_id")?
            .as_str()?
            != turn
    {
        return None;
    }
    if value
        .pointer("/payload/internal_chat_message_metadata_passthrough/content_item_kinds")
        .and_then(Value::as_array)
        .is_some_and(|kinds| !kinds.iter().any(|kind| kind == "user.text"))
    {
        return None;
    }
    let content = value["payload"]["content"].as_array()?;
    if content.len() != 1 || content[0]["type"] != "input_text" {
        return None;
    }
    let text = content[0]["text"].as_str()?;
    if text.len() > MAX_PROVIDER_PROMPT_BYTES || crate::notice::parse_notice(text).is_none() {
        return None;
    }
    Some(Row {
        native_uuid: String::new(),
        text: text.into(),
        timestamp: value["timestamp"].as_str().map(str::to_owned),
    })
}

fn user_response(value: &Value) -> bool {
    value["type"] == "response_item"
        && value["payload"]["type"] == "message"
        && value["payload"]["role"] == "user"
}

fn snapshot(
    baseline: &mut Baseline,
    id: &str,
    text: &str,
    turn: &str,
    provider: &str,
    deadline: Instant,
) -> Result<Option<Proof>, ()> {
    use std::os::unix::fs::MetadataExt;
    if Instant::now() >= deadline {
        return Err(());
    }
    let metadata = baseline.file.metadata().map_err(|_| ())?;
    if !opened_file_still_current(&baseline.path, file_identity(&metadata))
        || (metadata.dev(), metadata.ino()) != (baseline.stamp.device, baseline.stamp.inode)
        || metadata.len() < baseline.stamp.eof
        || metadata.len() - baseline.stamp.eof > 1024 * 1024
    {
        return Err(());
    }
    if sha256(&read_continuity(&mut baseline.file, baseline.stamp.eof).map_err(|_| ())?)
        != baseline.stamp.continuity_sha256
    {
        return Err(());
    }
    baseline
        .file
        .seek(SeekFrom::Start(baseline.stamp.eof))
        .map_err(|_| ())?;
    let mut bytes = vec![0; (metadata.len() - baseline.stamp.eof) as usize];
    baseline.file.read_exact(&mut bytes).map_err(|_| ())?;
    if bytes.last().is_some_and(|byte| *byte != b'\n') {
        return Ok(None);
    }
    let mut candidate = None;
    let mut found = None;
    let mut offset = baseline.stamp.eof;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if Instant::now() >= deadline || line.len() > MAX_PROVIDER_LINE_BYTES {
            return Err(());
        }
        let current = offset;
        offset += line.len() as u64;
        let payload = native_notice::payload(line);
        let value: Value = serde_json::from_slice(payload).map_err(|_| ())?;
        if client(&value) == Some(id) {
            if found.is_some() || !event_matches(&value, id, text, turn, provider) {
                return Err(());
            }
            let (row_offset, row, row_payload_sha256) = candidate.take().ok_or(())?;
            found = Some(Proof {
                row,
                row_offset,
                row_payload_sha256,
                codex: Some(store::codex::Row {
                    event_offset: current,
                    event_payload_sha256: sha256(payload),
                    turn_id: turn.into(),
                }),
            });
        } else if found.is_none() && user_response(&value) {
            if candidate.is_some() {
                return Err(());
            }
            let mut row = response(&value, turn)
                .filter(|row| row.text == text)
                .ok_or(())?;
            row.native_uuid = sha256(payload);
            candidate = Some((current, row, sha256(payload)));
        } else if found.is_none()
            && value["type"] == "event_msg"
            && value["payload"]["type"] == "user_message"
        {
            return Err(());
        }
    }
    Ok(found)
}

pub(crate) fn confirm(
    baseline: &mut Baseline,
    id: &str,
    text: &str,
    turn: &str,
    provider: &str,
    deadline: Instant,
) -> Option<Proof> {
    loop {
        match snapshot(baseline, id, text, turn, provider, deadline) {
            Ok(Some(proof)) => return Some(proof),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            ),
            _ => return None,
        }
    }
}

pub(crate) fn project(
    path: &Path,
    value: &Value,
    claim: &store::Claim,
    proof: &store::Confirmed,
) -> Option<Row> {
    let codex = claim.codex.as_ref()?;
    let association = proof.codex.as_ref()?;
    if association.event_offset <= proof.row_offset
        || association.event_offset - proof.row_offset > 1024 * 1024
    {
        return None;
    }
    let mut row = response(value, &association.turn_id)?;
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(association.event_offset)).ok()?;
    let mut line = Vec::new();
    BufReader::new(file.take(MAX_PROVIDER_LINE_BYTES as u64 + 1))
        .read_until(b'\n', &mut line)
        .ok()?;
    if line.last() != Some(&b'\n') || line.len() > MAX_PROVIDER_LINE_BYTES {
        return None;
    }
    let payload = native_notice::payload(&line);
    if sha256(payload) != association.event_payload_sha256 {
        return None;
    }
    let event = serde_json::from_slice(payload).ok()?;
    if !event_matches(
        &event,
        &codex.client_message_id,
        &row.text,
        &association.turn_id,
        &claim.binding.provider_session_id,
    ) {
        return None;
    }
    row.native_uuid = proof.row_payload_sha256.clone();
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn closed_notice_slice_requires_unique_client_event_response_and_turn() {
        let text =
            "<!-- agent-notice:v1 kind=runtime.resume source=agent-session -->\n\nContinue  \n";
        let row = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user",
            "content":[{"type":"input_text","text":text}],
            "internal_chat_message_metadata_passthrough":{"turn_id":"accepted-turn"}}});
        let event = serde_json::json!({"type":"event_msg","payload":{"type":"user_message","client_id":"agent-session-fixture","message":text}});
        for mode in [
            "exact",
            "missing",
            "different_id",
            "duplicate_id",
            "interleaved",
            "missing_turn",
            "different_turn",
            "changed_text",
            "human_after",
            "canonical",
            "foreign_thread",
            "context_kind",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("rollout.jsonl");
            fs::write(&path, "{}\n").unwrap();
            let mut baseline = Baseline::pin(path.clone()).unwrap();
            let mut response = row.clone();
            let mut accepted = event.clone();
            match mode {
                "different_id" => {
                    accepted["payload"]["client_id"] = serde_json::json!("agent-session-other")
                }
                "missing_turn" => {
                    response["payload"]
                        .as_object_mut()
                        .unwrap()
                        .remove("internal_chat_message_metadata_passthrough");
                }
                "different_turn" => {
                    response["payload"]["internal_chat_message_metadata_passthrough"]["turn_id"] =
                        serde_json::json!("other-turn")
                }
                "changed_text" => accepted["payload"]["message"] = serde_json::json!("other words"),
                "canonical" | "foreign_thread" => {
                    accepted = serde_json::json!({"type":"event_msg","payload":{"type":"item_completed",
                    "thread_id":if mode=="canonical" {"thread-one"} else {"foreign-thread"},"turn_id":"accepted-turn",
                    "item":{"type":"UserMessage","id":"item-one","client_id":"agent-session-fixture","content":[{"type":"text","text":text}]}}})
                }
                "context_kind" => {
                    response["payload"]["internal_chat_message_metadata_passthrough"]["content_item_kinds"] =
                        serde_json::json!(["context.other"])
                }
                _ => {}
            }
            let mut suffix = format!("{response}\n");
            if mode == "interleaved" {
                suffix.push_str(&format!("{row}\n"));
            }
            if mode != "missing" {
                suffix.push_str(&format!("{accepted}\n"));
            }
            if mode == "duplicate_id" {
                suffix.push_str(&format!("{event}\n"));
            }
            if mode == "human_after" {
                suffix.push_str(&format!("{row}\n"));
            }
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(suffix.as_bytes())
                .unwrap();
            let proof = snapshot(
                &mut baseline,
                "agent-session-fixture",
                text,
                "accepted-turn",
                "thread-one",
                Instant::now() + Duration::from_secs(1),
            );
            let proved = proof.as_ref().is_ok_and(|proof| proof.is_some());
            assert_eq!(
                proved,
                matches!(mode, "exact" | "human_after" | "canonical"),
                "{mode}"
            );
            if let Ok(Some(proof)) = proof {
                assert_eq!(proof.row.text, text);
                assert_eq!(proof.row_offset, 3);
            }
        }
    }
}
