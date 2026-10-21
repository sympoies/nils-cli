//! A prompt Claude takes into a running tool-call turn is a delivery, even
//! though the queue is empty again by the time the acknowledgement wait ends.
use super::super::*;

const DRAINED_CLAUDE_PANE: &str = "● Ran the next tool call.
────────────────────────────────────────
❯\u{a0}
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents
";

/// Milliseconds since the epoch; BSD `date` has no `%N`, so the stub asks Python.
const MILLIS_NOW: &str = "python3 -c 'import time; print(int(time.time() * 1000))'";

/// A tmux stub whose pane lists the prompt as queued for `queued_for` after the
/// Enter that submits it, and shows the drained composer from then on.
fn queue_draining_tmux(dir: &Path, queued_for: Duration, queued_pane: &str) -> PathBuf {
    let bin = minimal_tmux(dir);
    let queued = dir.join("queued-pane");
    let drained = dir.join("drained-pane");
    std::fs::write(&queued, queued_pane).unwrap();
    std::fs::write(&drained, DRAINED_CLAUDE_PANE).unwrap();
    let entered = dir.join("entered-at");
    let script = std::fs::read_to_string(&bin).unwrap();
    let script = script
        .replace(
            "  capture-pane) printf 'pane\\n'; exit 0 ;;",
            &format!(
                "  send-keys) case \"$*\" in *Enter*) {now} > '{entered}' ;; esac; exit 0 ;;\n  capture-pane) if [ -f '{entered}' ] && [ $(( $({now}) - $(cat '{entered}') )) -lt {ms} ]; then cat '{queued}'; else cat '{drained}'; fi; exit 0 ;;",
                now = MILLIS_NOW,
                entered = entered.display(),
                queued = queued.display(),
                drained = drained.display(),
                ms = queued_for.as_millis(),
            ),
        );
    std::fs::write(&bin, script).unwrap();
    bin
}

async fn submit(
    tmp: &Path,
    id: &str,
    text: &str,
    queued_for: Duration,
    ack_wait: Duration,
) -> (StatusCode, Value, String) {
    let launch_id = seed_claude_prompt_session(tmp, id);
    let st = state(
        tmp,
        Some(TOKEN),
        queue_draining_tmux(tmp, queued_for, QUEUED_CLAUDE_PANE),
    );
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (status, body) = response_parts(
        submit_claude_structured_prompt(&st, &record, &launch_id, Some(&launch_id), text, ack_wait)
            .await,
    )
    .await;
    (status, body, launch_id)
}

#[tokio::test]
async fn queued_prompt_that_leaves_the_queue_without_a_new_turn_is_submitted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (status, body, launch_id) = submit(
        tmp.path(),
        "claude-drained",
        "Second queued message.",
        QUEUE_WINDOW,
        GENEROUS_ACK_WAIT,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
    assert_eq!(body["data"]["queued"], true);
    assert_eq!(body["data"]["session_incarnation"], launch_id);
}

#[tokio::test]
async fn prompt_absent_from_the_queue_stays_unknown() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (status, body, _) = submit(
        tmp.path(),
        "claude-never-queued",
        "A prompt the queue never received",
        QUEUE_WINDOW,
        Duration::from_millis(1500),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
    assert!(body.pointer("/data/submitted").is_none());
}

/// How long the stub lists a prompt as queued. A pane read on a loaded host can
/// start late, so the window is wide; tests that need the queue empty again poll
/// for it instead of sleeping.
const QUEUE_WINDOW: Duration = Duration::from_secs(3);
/// A wait that ends at the first queued sighting, so it only bounds a hang.
const GENEROUS_ACK_WAIT: Duration = Duration::from_secs(30);

const TWO_QUEUED_CLAUDE_PANE: &str = "  First queued hand-off.
  Second queued hand-off.
  ctrl+x ctrl+s to send now
· Galloping… (running PreToolUse hook · 4s)
────────────────────────────────────────
❯\u{a0}Press up to edit queued messages
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents
";

#[tokio::test]
async fn consecutive_hand_offs_in_one_tool_call_turn_are_each_submitted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-two-hand-offs";
    let launch_id = seed_claude_prompt_session(tmp.path(), id);
    let st = state(
        tmp.path(),
        Some(TOKEN),
        queue_draining_tmux(tmp.path(), QUEUE_WINDOW, TWO_QUEUED_CLAUDE_PANE),
    );
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    for text in ["First queued hand-off.", "Second queued hand-off."] {
        let (status, body) = response_parts(
            submit_claude_structured_prompt(
                &st,
                &record,
                &launch_id,
                Some(&launch_id),
                text,
                GENEROUS_ACK_WAIT,
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}: body={body}");
        assert_eq!(body["data"]["queued"], true, "{text}");
        assert_eq!(body["data"]["session_incarnation"], launch_id);
        // The first hand-off's queue window closes before the next one starts.
        while crate::claude_prompt_queued(&record, text, &st.tmux_bin) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
