//! Receipt-bound launch rows are excluded from human prompt previews.
use super::*;

pub(super) fn enrich_launch_previews(
    sessions: &mut [HistorySession],
    cache: &mut HashMap<PathBuf, LatestPromptCacheEntry>,
    state: Option<&Path>,
) {
    enrich_latest_prompt_previews(sessions, cache);
    let Some(state) = state else { return };
    let budget =
        LATEST_PREVIEW_SESSION_MAX_BYTES.min(LATEST_PREVIEW_PAGE_MAX_BYTES / sessions.len().max(1));
    for session in sessions {
        let deadline = Instant::now() + SCAN_MAX_DURATION;
        let Some(binding) = crate::notice::launch::binding(state, session, deadline) else {
            continue;
        };
        let first = first_without_launch(session, &binding, deadline);
        session.first_user_prompt_preview = first.clone();
        session.prompt_preview = first;
        session.last_user_prompt_preview = ProviderPromptSource::for_history(
            &session.provider,
            session.provider_session_id.clone(),
            session.transcript_path.clone(),
        )
        .and_then(|source| {
            crate::provider_prompt::read_last_prompt_skipping_launch(&source, budget, &binding)
        })
        .map(|prompt| truncate_chars(prompt.text.trim(), MAX_PREVIEW_CHARS));
    }
}

fn first_without_launch(
    session: &HistorySession,
    binding: &(u64, String, String),
    deadline: Instant,
) -> Option<String> {
    let mut reader = BufReader::new(fs::File::open(&session.transcript_path).ok()?);
    let mut line = Vec::new();
    for _ in 0..MAX_PREVIEW_LINES {
        if Instant::now() >= deadline {
            return None;
        }
        let offset = reader.stream_position().ok()?;
        match read_bounded_line(&mut reader, &mut line).ok()? {
            BoundedLine::Eof => return None,
            BoundedLine::Oversized => continue,
            BoundedLine::Line => {}
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&line)
            && crate::notice::launch::bound_row(binding, offset, &line, &value)
        {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        if let Some(prompt) =
            history_preview_prompt(&session.provider, &session.provider_session_id, text)
        {
            return Some(truncate_chars(prompt.text.trim(), MAX_PREVIEW_CHARS));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_only_tail_has_no_human_preview_but_unclaimed_paste_does() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let notice = "<!-- agent-notice:v1 kind=example source=fixture -->\n\nbody\n";
        let wrapped =
            format!("\n\n<pasted_content id=\"abcd\">\n{notice}</pasted_content id=\"abcd\">\n");
        let line = serde_json::json!({"type":"user", "sessionId":"fixture", "message":{"role":"user", "content":wrapped}}).to_string();
        fs::write(&path, format!("{line}\n")).unwrap();
        let source = ProviderPromptSource::for_history("claude", "fixture", path).unwrap();
        let binding = (
            0,
            notice.to_owned(),
            crate::send::claude_notice::sha256(line.as_bytes()),
        );
        assert!(
            crate::provider_prompt::read_last_prompt_skipping_launch(&source, 65536, &binding)
                .is_none()
        );
        let unclaimed = (1, binding.1.clone(), binding.2.clone());
        assert!(
            crate::provider_prompt::read_last_prompt_skipping_launch(&source, 65536, &unclaimed)
                .is_some()
        );
    }
}
