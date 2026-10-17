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
fn queue_draining_tmux(dir: &Path, queued_for: Duration) -> PathBuf {
    let bin = minimal_tmux(dir);
    let queued = dir.join("queued-pane");
    let drained = dir.join("drained-pane");
    std::fs::write(&queued, QUEUED_CLAUDE_PANE).unwrap();
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
) -> (StatusCode, Value, String) {
    let launch_id = seed_claude_prompt_session(tmp, id);
    let st = state(tmp, Some(TOKEN), queue_draining_tmux(tmp, queued_for));
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch_id,
            Some(&launch_id),
            text,
            Duration::from_millis(1500),
        )
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
        Duration::from_millis(800),
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
        Duration::from_millis(800),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
    assert!(body.pointer("/data/submitted").is_none());
}
