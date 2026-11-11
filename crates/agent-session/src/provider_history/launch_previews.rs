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
        let notices =
            match history_images::native_notices::Bindings::load(Some(state), session, deadline) {
                Ok(notices) => notices,
                Err(_) => continue,
            };
        if notices.has_cutoff() {
            apply_cutoff_previews(session, &notices, budget, deadline);
        }
    }
}

fn apply_cutoff_previews(
    session: &mut HistorySession,
    notices: &history_images::native_notices::Bindings,
    budget: usize,
    deadline: Instant,
) {
    let first = cutoff_preview_scan(session, notices, 0, MAX_PREVIEW_LINES, deadline).0;
    session.first_user_prompt_preview = first.clone();
    session.prompt_preview = first;
    if let Ok(metadata) = fs::metadata(&session.transcript_path) {
        let start = metadata.len().saturating_sub(budget as u64);
        let (_, last) = cutoff_preview_scan(session, notices, start, usize::MAX, deadline);
        if let Some((last, true)) = &last {
            session.last_user_prompt_preview = Some(last.clone());
        } else if notices.launch_binding().is_some() && last.is_none() {
            session.last_user_prompt_preview = None;
        }
    }
}

fn cutoff_preview_scan(
    session: &HistorySession,
    notices: &history_images::native_notices::Bindings,
    start: u64,
    max_lines: usize,
    deadline: Instant,
) -> (Option<String>, Option<(String, bool)>) {
    let Ok(mut file) = fs::File::open(&session.transcript_path) else {
        return (None, None);
    };
    let Ok(metadata) = file.metadata() else {
        return (None, None);
    };
    let available = metadata.len().saturating_sub(start);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (None, None);
    }
    let mut reader = BufReader::new(file.take(available));
    let mut line = Vec::new();
    let mut first = None;
    let mut last = None;
    for _ in 0..max_lines {
        if Instant::now() >= deadline {
            break;
        }
        let offset = start + available - reader.get_ref().limit() - reader.buffer().len() as u64;
        match read_bounded_line(&mut reader, &mut line) {
            Ok(BoundedLine::Eof) | Err(_) => break,
            Ok(BoundedLine::Oversized) => continue,
            Ok(BoundedLine::Line) => {}
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if !matches!(
            notices.project(session, offset, &line, &value, false),
            history_images::native_notices::Decision::NotBound
        ) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        let prompt = if notices.legacy_header(offset) {
            history_preview_prompt(&session.provider, &session.provider_session_id, text)
        } else {
            let parsed = crate::provider_prompt::parse_history_user_prompt(
                &session.provider,
                &session.provider_session_id,
                text,
            );
            parsed.filter(|prompt| {
                !crate::provider_prompt::row_classes::notice::matches(
                    &session.provider,
                    &prompt.text,
                    &value,
                ) || (prompt.text.starts_with("<!-- agent-notice:")
                    && crate::notice::parse_notice(&prompt.text).is_some())
            })
        };
        if let Some(prompt) = prompt {
            let text = truncate_chars(prompt.text.trim(), MAX_PREVIEW_CHARS);
            if first.is_none() {
                first = Some(text.clone());
            }
            let post_cutoff_header = !notices.legacy_header(offset)
                && prompt.text.starts_with("<!-- agent-notice:")
                && crate::notice::parse_notice(&prompt.text).is_some();
            last = Some((text, post_cutoff_header));
        }
    }
    (first, last)
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
