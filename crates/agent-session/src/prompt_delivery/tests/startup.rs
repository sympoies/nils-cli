//! Exercise the same staged-file delivery boundary used by `start --prompt`.
use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    dir: tempfile::TempDir,
    tmux: PathBuf,
    record: SessionRecord,
}

impl Fixture {
    fn new(screen: &str, fail_paste: bool) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let prompt = root.join("prompt.md");
        fs::write(&prompt, "synthetic initial prompt").unwrap();
        let tmux = root.join("tmux");
        // Input is accepted into a pending composer immediately, but rendering
        // may lag beyond the settle interval. Enter commits all pending bytes.
        fs::write(
            &tmux,
            format!(
                r#"#!/bin/sh
root={root}
printf '%s\n' "$1" >> "$root/calls"
case "$1" in
  display-message) echo '1|2' ;;
  capture-pane)
    case {screen} in
      unavailable) exit 1 ;;
      changed) [ ! -f "$root/pending" ] || {{ echo 'updated pane'; exit 0; }} ;;
    esac
    printf '────────────────────\n❯ \n────────────────────\n'
    ;;
  load-buffer) cp "$4" "$root/buffer" ;;
  paste-buffer)
    cat "$root/buffer" >> "$root/pending"
    {paste_result}
    ;;
  send-keys) cp "$root/pending" "$root/first-turn" ;;
esac
"#,
                root = shell_words::quote(&root.to_string_lossy()),
                screen = shell_words::quote(screen),
                paste_result = if fail_paste { "exit 1" } else { "exit 0" },
            ),
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        let record = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": "startup-fixture", "agent": "claude", "mode": "interactive",
            "cwd": root, "tmux_session": "startup-fixture",
            "prompt_file": prompt,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z"
        }))
        .unwrap();
        Self { dir, tmux, record }
    }

    fn calls(&self, command: &str) -> usize {
        fs::read_to_string(self.dir.path().join("calls"))
            .unwrap()
            .lines()
            .filter(|line| *line == command)
            .count()
    }
}

#[test]
fn startup_initial_prompt_is_not_replayed_when_accepted_before_render() {
    let fixture = Fixture::new("unchanged", false);
    let observation = crate::paste_prompt(&fixture.tmux, &fixture.record).unwrap();
    assert_eq!(observation.composer.state, "unchanged_after_paste");
    assert_eq!(
        fs::read_to_string(fixture.dir.path().join("first-turn")).unwrap(),
        "synthetic initial prompt",
        "unchanged pixels cannot authorize another copy of accepted input"
    );
    assert_eq!(fixture.calls("paste-buffer"), 1);
    assert_eq!(fixture.calls("send-keys"), 1);
}

#[test]
fn startup_changed_or_unavailable_observation_does_not_replay_input() {
    for screen in ["changed", "unavailable"] {
        let fixture = Fixture::new(screen, false);
        crate::paste_prompt(&fixture.tmux, &fixture.record).unwrap();
        assert_eq!(fixture.calls("paste-buffer"), 1);
        assert_eq!(fixture.calls("send-keys"), 1);
        assert_eq!(
            fs::read_to_string(fixture.dir.path().join("first-turn")).unwrap(),
            "synthetic initial prompt"
        );
    }
}

#[test]
fn startup_uncertain_paste_failure_never_replays_or_submits() {
    let fixture = Fixture::new("unchanged", true);
    assert!(crate::paste_prompt(&fixture.tmux, &fixture.record).is_err());
    assert_eq!(fixture.calls("paste-buffer"), 1);
    assert_eq!(fixture.calls("delete-buffer"), 1);
    assert_eq!(fixture.calls("send-keys"), 0);
    assert!(!fixture.dir.path().join("first-turn").exists());
}
