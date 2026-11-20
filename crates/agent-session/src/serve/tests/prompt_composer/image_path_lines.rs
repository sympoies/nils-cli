//! LF framing and guarded submission of Claude image-path prompts.
use super::super::*;
use pretty_assertions::assert_eq;

const PROMPT: &str = "\"sample image.png\"\n> Describe the image.";
const RENDERED: &str = "[Image #1]> Describe the image.";
const ID: &str = "image-path-lines";

/// Model the Claude composer only after a bracketed paste with intact LF rows.
/// The startup paste-byte tests verify tmux's actual interpretation of the flags.
fn image_path_tmux(dir: &Path, working: bool) -> (PathBuf, PathBuf, PathBuf) {
    let (bin, _) = recording_tmux(dir);
    let entered = dir.join("enters");
    let rendered = dir.join("rendered");
    fs::write(dir.join("expected"), PROMPT).unwrap();
    fs::write(
        dir.join("after-enter"),
        if working {
            QUEUED_CLAUDE_PANE.replace("Second queued message.", RENDERED)
        } else {
            String::from("────────────────────\n❯ \n────────────────────\n")
        },
    )
    .unwrap();
    let script = fs::read_to_string(&bin).unwrap().replacen(
        "capture-pane) printf 'pane\\n'; exit 0 ;;",
        &format!(
            r#"load-buffer)
    for file do :; done
    cp "$file" "$fixture/staged"
    exit $? ;;
  paste-buffer)
    case " $* " in *" -p "*) ;; *) exit 0 ;; esac
    case " $* " in *" -r "*) ;; *) exit 0 ;; esac
    cmp -s "$fixture/expected" "$fixture/staged" && touch "$fixture/rendered"
    exit 0 ;;
  send-keys)
    for key do :; done
    if [ "$key" = Enter ]; then printf 'Enter\n' >> "$fixture/enters"; fi
    exit 0 ;;
  capture-pane)
    if [ -f "$fixture/enters" ]; then
      cat "$fixture/after-enter"
    elif [ -f "$fixture/rendered" ]; then
      printf '────────────────────\n❯ %s\n────────────────────\n' '{RENDERED}'
    else
      printf '────────────────────\n❯ \n────────────────────\n'
    fi
    exit 0 ;;"#,
        ),
        1,
    );
    let dir = dir.to_string_lossy();
    let script = script.replacen(
        "#!/usr/bin/env sh\n",
        &format!("#!/usr/bin/env sh\nfixture={}\n", shell_words::quote(&dir)),
        1,
    );
    fs::write(&bin, script).unwrap();
    (bin, rendered, entered)
}

async fn submit_image_path(working: bool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("sample image.png"), b"synthetic").unwrap();
    let launch = seed_claude_prompt_session(dir, ID);
    let (tmux, rendered, entered) = image_path_tmux(dir, working);
    let state = state(dir, Some(TOKEN), tmux);
    let mut record = load_session_record(&state.context, ID).unwrap();
    record.cwd = dir.to_str().unwrap().to_owned();
    crate::write_session_record(&state.context, &record).unwrap();
    crate::activity::activate_runtime(&state.context, &record).unwrap();
    let start = claude_turn_started_event(&launch, "seed-start", "seed-turn");
    crate::activity::ingest_event(&state.context, ID, start).unwrap();
    if !working {
        let stop = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": "seed-end",
            "runtime_id": launch,
            "provider": "claude",
            "provider_turn_id": "seed-turn",
            "kind": "turn_completed",
            "confidence": "authoritative"
        }))
        .unwrap();
        crate::activity::ingest_event(&state.context, ID, stop).unwrap();
    }

    let acknowledger = (!working).then(|| {
        let entered = entered.clone();
        let context = state.context.clone();
        let start = claude_turn_started_event(&launch, "image-start", "image-turn");
        tokio::spawn(async move {
            wait_for_file(&entered).await;
            tokio::task::spawn_blocking(move || {
                crate::activity::ingest_event(&context, ID, start).unwrap();
            })
            .await
            .unwrap();
        })
    });
    let (status, body) = call(
        router(state),
        post_json(
            "/sessions/image-path-lines/prompt/v2",
            Some(TOKEN),
            json!({
                "text": PROMPT,
                "expected_session_incarnation": launch,
                "request_id": "image-path-lines",
            }),
        ),
    )
    .await;
    if let Some(acknowledger) = acknowledger {
        if status == StatusCode::OK {
            acknowledger.await.unwrap();
        } else {
            acknowledger.abort();
        }
    }

    assert!(rendered.exists(), "image draft requires bracketed LF rows");
    assert_eq!(
        fs::read_to_string(&entered).unwrap_or_default(),
        "Enter\n",
        "expected exactly one Enter for the rendered image draft, response {status} {body}"
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["submitted"], true);
    assert_eq!(body["data"]["queued"].as_bool().unwrap_or(false), working);
}

#[tokio::test]
async fn idle_prompt_v2_submits_lf_framed_image_path_once() {
    submit_image_path(false).await;
}

#[tokio::test]
async fn mid_turn_prompt_v2_queues_lf_framed_image_path_once() {
    submit_image_path(true).await;
}
