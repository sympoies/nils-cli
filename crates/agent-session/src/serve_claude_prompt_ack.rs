//! Delivery proof for a structured prompt pasted into a Claude pane.
use super::*;

/// What the pane and the provider hook showed after the prompt was submitted.
pub(super) enum ClaudePromptAck {
    /// Claude's own hook reported a turn the baseline did not contain.
    Turn,
    /// The pane or transcript listed the prompt among its queued messages. A running turn
    /// takes the queue into itself at its next tool call, so the sighting is
    /// the proof; the queue may well be empty again by the time the wait ends.
    Queued,
    /// Neither was seen: the text may or may not have reached Claude.
    Unknown,
}

/// Watch for the provider's turn event, newly appended queue records, and
/// new full-text pane queue sightings until `wait` ends. Both baselines start
/// before submission, so an older identical prompt cannot acknowledge it.
pub(super) async fn await_claude_prompt_ack(
    state: &Arc<ServeState>,
    record: &SessionRecord,
    baseline: &crate::activity::PromptTurnBaseline,
    mut prompt_tail: Option<ProviderPromptTail>,
    queue_before: Option<&str>,
    text: &str,
    wait: Duration,
) -> Result<ClaudePromptAck, ()> {
    let queue_count_before =
        queue_before.and_then(|pane| crate::send_submit::claude_queue_occurrences(pane, text));
    let deadline = Instant::now() + wait;
    loop {
        let context = state.context.clone();
        let observed = (record.clone(), baseline.clone(), text.to_string());
        let tmux_bin = state.tmux_bin.clone();
        let mut tail = prompt_tail.take();
        let (ack, tail) = tokio::task::spawn_blocking(move || {
            let (record, baseline, text) = observed;
            let transcript_queued = tail.as_mut().is_some_and(|tail| {
                tail.poll_queued_prompts()
                    .is_ok_and(|prompts| prompts.iter().any(|prompt| prompt == &text))
            });
            let pane_queued = queue_count_before.is_some_and(|before| {
                crate::claude_prompt_queue_count(&record, &text, &tmux_bin)
                    .is_some_and(|now| now > before)
            });
            let ack = if crate::activity::prompt_turn_started_since(&context, &record, &baseline) {
                ClaudePromptAck::Turn
            } else if transcript_queued || pane_queued {
                ClaudePromptAck::Queued
            } else {
                ClaudePromptAck::Unknown
            };
            (ack, tail)
        })
        .await
        .map_err(|_| ())?;
        prompt_tail = tail;
        if !matches!(ack, ClaudePromptAck::Unknown) || Instant::now() >= deadline {
            return Ok(ack);
        }
        tokio::time::sleep(CLAUDE_STRUCTURED_PROMPT_ACK_POLL_INTERVAL).await;
    }
}
