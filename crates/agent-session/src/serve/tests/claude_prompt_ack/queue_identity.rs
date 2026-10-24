// Sibling queue text cannot acknowledge a later lost paste.
use super::*;
use pretty_assertions::assert_eq;

const FIRST: &str = "Inspect the shared hand-off prefix: first task.";
const SECOND: &str = "Inspect the shared hand-off prefix: second task.";

fn queue_pane(text: &str) -> String {
    QUEUED_CLAUDE_PANE.replace("Second queued message.", text)
}

fn queue_transition_tmux(dir: &Path, before: &str, after: &str) -> PathBuf {
    let bin = minimal_tmux(dir);
    let before_path = dir.join("before-pane");
    let after_path = dir.join("after-pane");
    let entered = dir.join("entered");
    let calls = dir.join("input-calls");
    fs::write(&before_path, before).unwrap();
    fs::write(&after_path, after).unwrap();
    for (path, pane) in [(&before_path, before), (&after_path, after)] {
        fs::write(
            path.with_extension("styled"),
            pane.replace(
                "Press up to edit queued messages",
                "\x1b[2mPress up to edit queued messages\x1b[22m",
            ),
        )
        .unwrap();
    }
    let script = fs::read_to_string(&bin).unwrap().replace(
        "  capture-pane) printf 'pane\\n'; exit 0 ;;",
        &format!(
            "  load-buffer|paste-buffer|send-keys) echo \"$1\" >> '{}'; case \"$*\" in *Enter*) touch '{}' ;; esac; exit 0 ;;\n  capture-pane) if [ -f '{}' ]; then pane='{}'; else pane='{}'; fi; case \"$*\" in *' -e '*) cat \"$pane.styled\" ;; *) cat \"$pane\" ;; esac; exit 0 ;;",
            calls.display(), entered.display(), entered.display(), after_path.display(), before_path.display(),
        ),
    );
    fs::write(&bin, script).unwrap();
    bin
}

#[tokio::test]
async fn sibling_queue_prefix_stays_unknown_and_receipt_replay_never_resends() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-sibling-queue";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let pane = queue_pane(FIRST);
    let st = state(
        tmp.path(),
        Some(TOKEN),
        queue_transition_tmux(tmp.path(), &pane, &pane),
    );
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let request = || {
        post_json(
            &format!("/sessions/{id}/prompt/v2"),
            Some(TOKEN),
            json!({"text": SECOND, "expected_session_incarnation": launch, "request_id": "sibling-attempt"}),
        )
    };
    let (status, body) = call(router(st.clone()), request()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["details"]["receipt"]["outcome"], "unknown");
    let calls = fs::read(tmp.path().join("input-calls")).unwrap();
    let (replay_status, replay) = call(router(st), request()).await;
    assert_eq!(replay_status, status);
    assert_eq!(replay, body);
    assert_eq!(fs::read(tmp.path().join("input-calls")).unwrap(), calls);
    assert_eq!(
        String::from_utf8(calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["load-buffer", "paste-buffer", "send-keys"]
    );
}

#[tokio::test]
async fn new_sibling_queue_entry_confirms_each_consecutive_hand_off() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-new-sibling";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    // A remains queued while B is added, then an identical B is added again.
    for (before, after) in [
        (FIRST.to_string(), format!("{FIRST}\n  {SECOND}")),
        (
            format!("{FIRST}\n  {SECOND}"),
            format!("{FIRST}\n  {SECOND}\n  {SECOND}"),
        ),
    ] {
        let _ = fs::remove_file(tmp.path().join("entered"));
        let st = state(
            tmp.path(),
            Some(TOKEN),
            queue_transition_tmux(tmp.path(), &queue_pane(&before), &queue_pane(&after)),
        );
        let record = load_session_record(&st.context, id).unwrap();
        crate::activity::activate_runtime(&st.context, &record).unwrap();
        let (status, body) = response_parts(
            submit_claude_structured_prompt(
                &st,
                &record,
                &launch,
                Some(&launch),
                SECOND,
                Duration::from_millis(400),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["queued"], true);
    }
}

#[tokio::test]
async fn old_identical_queue_entry_stays_unknown() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-old-identical";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let pane = queue_pane(SECOND);
    let st = state(
        tmp.path(),
        Some(TOKEN),
        queue_transition_tmux(tmp.path(), &pane, &pane),
    );
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch,
            Some(&launch),
            SECOND,
            Duration::from_millis(400),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
}
