//! Structured Claude prompts are never left pasted without their Enter.
use super::super::*;
use pretty_assertions::assert_eq;

const ID: &str = "structured";

/// A stateful pane: empty until the paste, showing `composer` after it, and
/// empty again after the first Enter. `after_paste` runs inside the paste.
fn pasted_pane_tmux(dir: &Path, composer: &str, after_paste: &str) -> (PathBuf, PathBuf) {
    let (bin, journal) = recording_tmux(dir);
    let (pasted, entered) = (dir.join("pasted"), dir.join("entered"));
    let empty = r"printf '────────────────────\n❯ \n────────────────────\n'";
    let script = fs::read_to_string(&bin)
        .unwrap()
        .replacen(
            "capture-pane) printf 'pane\\n'; exit 0 ;;",
            &format!(
                "capture-pane) if [ -f '{entered}' ] || [ ! -f '{pasted}' ]; then {empty}; else printf '────────────────────\\n❯ %s\\n────────────────────\\n' '{composer}'; fi; exit 0 ;;",
                entered = entered.display(),
                pasted = pasted.display(),
            ),
            1,
        )
        .replacen(
            "case \"$1\" in",
            &format!(
                "case \"$1\" in\n  paste-buffer) touch '{pasted}'; {after_paste}; exit 0 ;;\n  send-keys) case \"$*\" in *Enter*) touch '{entered}' ;; esac; exit 0 ;;",
                pasted = pasted.display(),
                entered = entered.display(),
            ),
            1,
        );
    fs::write(&bin, script).unwrap();
    (bin, journal)
}

fn enters(journal: &Path) -> usize {
    fs::read_to_string(journal)
        .unwrap()
        .lines()
        .filter(|line| *line == "send-keys")
        .count()
}

fn deliver(
    dir: &Path,
    tmux: &Path,
    text: &str,
) -> (
    CliContext,
    Result<crate::prompt_delivery::ClaudePromptDelivery, CliError>,
) {
    // The observation spool refuses a state root behind a symlink, as the macOS
    // temporary directory is, so the fixture roots state at its physical path.
    let dir = &fs::canonicalize(dir).unwrap();
    let launch = seed_claude_prompt_session(dir, ID);
    let state = state(dir, Some(TOKEN), tmux.to_owned());
    let mut record = load_session_record(&state.context, ID).unwrap();
    record.cwd = dir.to_string_lossy().into_owned();
    crate::write_session_record(&state.context, &record).unwrap();
    crate::activity::activate_runtime(&state.context, &record).unwrap();
    let delivered = crate::prompt_delivery::deliver_claude_structured_prompt_locked(
        &state.context,
        ID,
        &launch,
        text,
        tmux,
    );
    (state.context.clone(), delivered)
}

#[test]
fn hook_ingress_after_the_paste_cannot_strand_the_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    // A hook writer takes the shared ingress fence once the text is pasted and
    // then waits for the record lock that the delivery holds.
    let holder = tmp.path().join("hook-writer.py");
    fs::write(
        &holder,
        r#"import fcntl, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
with (root / 'sessions/structured/.activity-ingress.lock').open('a+') as ingress:
    fcntl.flock(ingress, fcntl.LOCK_SH)
    (root / 'hook-held').touch()
    time.sleep(2)
"#,
    )
    .unwrap();
    let after_paste = format!(
        "python3 '{holder}' '{root}' >/dev/null 2>&1 </dev/null & i=0; while [ ! -f '{root}/hook-held' ] && [ $i -lt 5 ]; do sleep 0.1; i=$((i+1)); done",
        holder = holder.display(),
        root = tmp.path().display(),
    );
    let (tmux, journal) = pasted_pane_tmux(tmp.path(), "please continue", &after_paste);

    let (_, delivered) = deliver(tmp.path(), &tmux, "please continue");

    delivered.expect("a hook writer after the paste must not refuse the Enter");
    assert_eq!(enters(&journal), 1);
}

#[test]
fn image_path_followed_by_text_is_submitted_from_its_rendered_composer() {
    for (prompt, composer) in [
        (
            "shot.png\ndescribe the image",
            "[Image #1] describe the image",
        ),
        (
            "\"shot.png\" describe the image",
            "[Image #1] describe the image",
        ),
        // The shape observed live: the label abuts the quoted rest.
        (
            "\"shot.png\"\n> Describe the image.",
            "[Image #1]> Describe the image.",
        ),
        ("shot.png", "[Image #1]"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("shot.png"), b"synthetic").unwrap();
        let (tmux, journal) = pasted_pane_tmux(tmp.path(), composer, "true");

        let (_, delivered) = deliver(tmp.path(), &tmux, prompt);

        let delivered = delivered.unwrap_or_else(|e| panic!("{prompt:?}: {}", e.code()));
        assert_eq!(delivered.acknowledgement_text, composer, "{prompt:?}");
        assert_eq!(enters(&journal), 1, "{prompt:?}");
    }
}

#[test]
fn foreign_composer_content_is_never_submitted_and_the_failure_is_diagnosable() {
    for composer in [
        "[Image #1] something else",
        "[Image #1] [Image #2] describe the image",
        "unrelated draft describe the image",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("shot.png"), b"synthetic").unwrap();
        let (tmux, journal) = pasted_pane_tmux(tmp.path(), composer, "true");

        let (context, delivered) = deliver(tmp.path(), &tmux, "shot.png\ndescribe the image");

        assert!(delivered.is_err(), "{composer:?}");
        assert_eq!(enters(&journal), 0, "{composer:?}");
        // The paste reached the composer, so the operator-facing diagnostic
        // must show that the prompt is pending, without any prompt text.
        let bundle = serde_json::to_value(
            crate::diagnose::build(
                &context,
                &crate::cli::DiagnoseArgs {
                    format: nils_common::cli_contract::OutputFormat::Json,
                    limit: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let recent = bundle["observation"]["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 1, "{bundle}");
        assert_eq!(recent[0]["code"], "prompt-pasted-not-submitted");
        assert_eq!(recent[0]["stage"], "prompt-delivery");
        assert_eq!(recent[0]["severity"], "error");
        let spool = recent[0].to_string();
        assert!(
            !spool.contains("describe") && !spool.contains("shot"),
            "{spool}"
        );
    }
}

#[test]
fn an_unrecorded_observation_is_reported_and_the_refusal_is_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("shot.png"), b"synthetic").unwrap();
    let (tmux, journal) = pasted_pane_tmux(tmp.path(), "[Image #1] something else", "true");
    let (mut context, delivered) = deliver(tmp.path(), &tmux, "shot.png\ndescribe the image");
    let Err(error) = delivered else {
        panic!("a foreign composer must be refused");
    };
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(enters(&journal), 0);

    // The spool refuses a state root behind a symlink, as the macOS temporary
    // directory is.
    let link = tmp.path().join("state-link");
    std::os::unix::fs::symlink(&context.state_dir, &link).unwrap();
    context.state_dir = link;

    assert_eq!(
        crate::prompt_delivery::post_paste::try_record(&context, &record, &error),
        Err(
            "warning: prompt-pasted-not-submitted observation not recorded: Untrusted"
                .to_string()
        )
    );
    assert_eq!(enters(&journal), 0);
}

fn two_image_fixture(dir: &Path) {
    for name in ["shot.png", "other.png"] {
        fs::write(dir.join(name), b"synthetic").unwrap();
    }
}

#[test]
fn several_leading_image_paths_are_submitted_from_their_rendered_labels() {
    for (prompt, composer) in [
        (
            "'shot.png'\n'other.png'\n> Describe the images.",
            "[Image #1][Image #2]> Describe the images.",
        ),
        (
            "\"shot.png\"\n'other.png'\ndescribe the images",
            "[Image #3] [Image #4] describe the images",
        ),
        ("'shot.png'\n'other.png'", "[Image #1][Image #2]"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        two_image_fixture(tmp.path());
        let (tmux, journal) = pasted_pane_tmux(tmp.path(), composer, "true");

        let (_, delivered) = deliver(tmp.path(), &tmux, prompt);

        let delivered = delivered.unwrap_or_else(|e| panic!("{prompt:?}: {}", e.code()));
        assert_eq!(delivered.acknowledgement_text, composer, "{prompt:?}");
        assert_eq!(enters(&journal), 1, "{prompt:?}");
    }
}

#[test]
fn mismatched_image_label_counts_are_never_submitted() {
    for (prompt, composer) in [
        ("'shot.png'\n'other.png'\ndescribe", "[Image #1] describe"),
        (
            "'shot.png'\n'other.png'\ndescribe",
            "[Image #1][Image #3] describe",
        ),
        (
            "'shot.png'\n'other.png'\ndescribe",
            "[Image #1][Image #2][Image #3] describe",
        ),
        ("'shot.png'\ndescribe", "[Image #1][Image #2] describe"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        two_image_fixture(tmp.path());
        let (tmux, journal) = pasted_pane_tmux(tmp.path(), composer, "true");

        let (_, delivered) = deliver(tmp.path(), &tmux, prompt);

        assert!(delivered.is_err(), "{prompt:?} / {composer:?}");
        assert_eq!(enters(&journal), 0, "{prompt:?} / {composer:?}");
    }
}
