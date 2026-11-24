//! Drive the actual guarded CLI against delayed, wrapped synthetic composers.
use super::{CmdOutput, Fixture, fs};
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

const KEY: &str = "long-paste-attempt";
const GUARDS: &[&str] = &[
    "--require-idle",
    "--require-empty-composer",
    "--idempotency-key",
    KEY,
];

fn long_text(segments: usize) -> String {
    (0..segments)
        .map(|index| format!("segment{index:02}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn wrapped_fixture(provider: &str) -> Fixture {
    let fixture = Fixture::new(provider);
    for marker in ["wrap-80", "word-wrap", "bare-composer"] {
        fs::write(fixture.root().join(marker), "").unwrap();
    }
    if provider == "codex" {
        // The synthetic cursor indexes the wrapped rows after the header.
        let path = fixture.root().join("tmux-model");
        let script = fs::read_to_string(&path).unwrap();
        let render = "print('› ' + shown + '\\n' + rows";
        assert!(script.contains(render));
        fs::write(
            &path,
            script.replace(render, "print('header\\n› ' + shown + '\\n' + rows"),
        )
        .unwrap();
    }
    fixture
}

fn delay_paste(fixture: &Fixture, first_chunk: usize) {
    let path = fixture.root().join("tmux-model");
    let script = fs::read_to_string(&path).unwrap();
    let paste = "if (root / 'late-render').exists(): (root / 'late').write_text((root / 'buffer').read_text())";
    let reveal = "if n >= 3:";
    assert!(script.contains(paste));
    assert!(script.contains(reveal));
    let script = script
        .replace(paste, &format!(
            "if (root / 'late-render').exists(): (root / 'composer').write_text((root / 'buffer').read_text()[:{first_chunk}]); (root / 'late').write_text((root / 'divergent-tail').read_text() if (root / 'divergent-tail').exists() else (root / 'buffer').read_text()[{first_chunk}:]); (root / 'render-start').write_text(str(time.monotonic()))"
        ))
        .replace(reveal, "if not (root / 'stall-paste').exists() and time.monotonic() - float((root / 'render-start').read_text()) >= 0.85:");
    fs::write(&path, script).unwrap();
    fs::write(fixture.root().join("late-render"), "").unwrap();
}

fn enter_count(fixture: &Fixture) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter"))
        .count()
}

fn assert_submitted_once(fixture: &Fixture, text: &str) {
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions")).unwrap(),
        format!("{}\n", serde_json::to_string(text).unwrap())
    );
    assert_eq!(enter_count(fixture), 1);
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        ""
    );
}

#[test]
fn long_paste_settles_before_one_submit_on_both_providers() {
    crate::cases::for_each_case(
        &[
            ("codex", 60, 0),
            ("codex", 60, 69),
            ("claude", 60, 69),
            ("claude", 120, 159),
        ],
        |&(provider, segments, first_chunk)| {
            let fixture = wrapped_fixture(provider);
            delay_paste(&fixture, first_chunk);
            let text = long_text(segments);
            let output = fixture.send_text(Some(&text), GUARDS);
            assert_eq!(
                output.code,
                0,
                "{provider} {segments} {first_chunk}: {}",
                output.stdout_text()
            );
            assert_submitted_once(&fixture, &text);
            assert!(
                fixture.calls().iter().any(|call| {
                    call[0] == "paste-buffer" && call.iter().any(|arg| arg == "-p")
                })
            );
        },
    );
}

#[test]
fn long_paste_stalled_prefix_refuses_without_enter() {
    let fixture = wrapped_fixture("codex");
    fs::write(fixture.root().join("stall-paste"), "").unwrap();
    delay_paste(&fixture, 69);
    let text = long_text(60);
    let output = fixture.send_text(Some(&text), GUARDS);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    let error = &output.stdout_json()["error"];
    assert!(matches!(
        error["code"].as_str(),
        Some("send-guard-observation-timeout" | "send-composer-unobservable")
    ));
    assert_eq!(error["details"]["stage"], "submit");
    assert_eq!(error["details"]["pasted"], true);
    assert_eq!(enter_count(&fixture), 0);
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn long_paste_divergent_tail_refuses_without_enter() {
    let fixture = wrapped_fixture("claude");
    fs::write(fixture.root().join("divergent-tail"), " x").unwrap();
    delay_paste(&fixture, 69);
    let text = long_text(60);
    let output = fixture.send_text(Some(&text), GUARDS);
    let receipt = fixture.refused(&output, "send-composer-mismatch");
    assert_eq!(receipt["error"]["details"]["stage"], "submit");
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        format!("{} x", &text[..69])
    );
    assert_eq!(enter_count(&fixture), 0);
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn long_paste_never_submits_a_preexisting_human_draft() {
    for provider in ["codex", "claude"] {
        let fixture = wrapped_fixture(provider);
        let path = fixture.root().join("composer");
        fs::write(&path, "human draft").unwrap();
        fixture.refused(
            &fixture.send_text(Some(&long_text(60)), GUARDS),
            "send-composer-not-empty",
        );
        fixture.no_delivery();
        assert_eq!(fs::read_to_string(path).unwrap(), "human draft");
    }
}

fn attempt(fixture: &Fixture, action: &str, text: &str) -> CmdOutput {
    run_resolved(
        "agent-session",
        &[
            "--state-dir",
            fixture.root().join("state").to_str().unwrap(),
            "attempt",
            action,
            "guarded",
            "--expect-runtime",
            "runtime-one",
            "--idempotency-key",
            KEY,
            "--tmux-bin",
            fixture.root().join("tmux").to_str().unwrap(),
            "--text-stdin",
            "--format",
            "json",
        ],
        &CmdOptions::new()
            .with_cwd(fixture.root())
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_stdin_str(text),
    )
}

fn retain_attempt(provider: &str, text: &str) -> Fixture {
    let fixture = wrapped_fixture(provider);
    for marker in ["start-after-paste", "backspace-ignored"] {
        fs::write(fixture.root().join(marker), "").unwrap();
    }
    fixture.refused(
        &fixture.send_text(Some(text), GUARDS),
        "send-session-not-idle",
    );
    fixture.idle();
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        text
    );
    assert_eq!(enter_count(&fixture), 0);
    fixture
}

#[test]
fn long_paste_keyed_recovery_reports_and_submits_only_the_original_input() {
    for provider in ["codex", "claude"] {
        let text = long_text(60);
        let fixture = retain_attempt(provider, &text);
        let before_status = fixture.calls().len();
        let output = attempt(&fixture, "status", &text);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(output.stdout_json()["data"]["unsubmitted_attempt"], true);
        assert_eq!(output.stdout_json()["data"]["composer"], "wrapped_match");
        assert!(
            fixture.calls()[before_status..]
                .iter()
                .all(|call| call[0] != "send-keys")
        );
        assert!(!output.stdout_text().contains(&text));

        fixture.refused(
            &attempt(&fixture, "recover", "different input"),
            "attempt-recovery-unproven",
        );
        let draft = format!("{text} human draft");
        let composer = fixture.root().join("composer");
        fs::write(&composer, &draft).unwrap();
        let changed_status = attempt(&fixture, "status", &text);
        assert_eq!(changed_status.code, 0, "{}", changed_status.stdout_text());
        assert_eq!(
            changed_status.stdout_json()["data"]["unsubmitted_attempt"],
            false
        );
        fixture.refused(
            &attempt(&fixture, "recover", &text),
            "attempt-recovery-unproven",
        );
        assert_eq!(fs::read_to_string(&composer).unwrap(), draft);
        assert_eq!(enter_count(&fixture), 0);

        // The visible wrap can omit a human-appended space. Recovery still
        // has to prove the logical end before it may send any Enter.
        let hidden = format!("{text} ");
        fs::write(&composer, &hidden).unwrap();
        let refused = attempt(&fixture, "recover", &text);
        assert_eq!(refused.code, 1, "{}", refused.stdout_text());
        assert_eq!(fs::read_to_string(&composer).unwrap(), hidden);
        assert_eq!(enter_count(&fixture), 0);

        let fixture = retain_attempt(provider, &text);
        let output = attempt(&fixture, "recover", &text);
        assert_eq!(output.code, 0, "{provider}: {}", output.stdout_text());
        assert_eq!(output.stdout_json()["data"]["recovery_enter_sent"], true);
        fixture.refused(
            &attempt(&fixture, "recover", &text),
            "attempt-recovery-unproven",
        );
        assert_submitted_once(&fixture, &text);
    }
}

#[test]
fn long_paste_recovery_refuses_shifted_invisible_wrap_spaces() {
    let text = long_text(60);
    let fixture = retain_attempt("codex", &text);
    let visible: Vec<_> = text
        .as_bytes()
        .chunks(70)
        .map(|row| std::str::from_utf8(row).unwrap().trim_end_matches(' '))
        .collect();
    let mut actual = String::new();
    let mut positions = serde_json::Map::new();
    let mut pane = String::from("header\n");
    for (row, visible) in visible.into_iter().enumerate() {
        let prefix = if row == 0 { "› " } else { "  " };
        pane.push_str(&format!("{prefix}{visible}\n"));
        if row > 0 {
            // Move a hidden wrap separator to the following boundary. The
            // rendered cells, total length, and final cursor all stay equal.
            actual.push_str(match row {
                1 => "",
                2 => "  ",
                _ => " ",
            });
        }
        for (column, character) in visible.chars().enumerate() {
            actual.push(character);
            positions.insert(
                actual.len().to_string(),
                serde_json::json!([3 + column, row + 1]),
            );
        }
    }
    pane.push_str("\n  status");
    assert_eq!(actual.len(), text.len());
    assert_ne!(actual, text);
    let layout = serde_json::json!({
        "input": actual,
        "pane": pane,
        "start": [2, 1],
        "before_end": positions[&(actual.len() - 1).to_string()],
        "prefix_end": positions[&actual.len().to_string()],
        "cursor_positions": positions,
    });
    let composer = fixture.root().join("composer");
    fs::write(&composer, &actual).unwrap();
    fs::write(fixture.root().join("captured-layout"), layout.to_string()).unwrap();

    let output = attempt(&fixture, "recover", &text);
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    assert_eq!(fs::read_to_string(&composer).unwrap(), actual);
    assert_eq!(enter_count(&fixture), 0);
}

fn list_row(fixture: &Fixture, text: &str) -> serde_json::Value {
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            fixture.root().join("state").to_str().unwrap(),
            "list",
            "--format",
            "json",
        ],
        &CmdOptions::new()
            .with_cwd(fixture.root())
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env(
                "AGENT_SESSION_TMUX_BIN",
                fixture.root().join("tmux").to_str().unwrap(),
            ),
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert!(!output.stdout_text().contains(KEY));
    assert!(!output.stdout_text().contains(text));
    output.stdout_json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "guarded")
        .unwrap()
        .clone()
}

#[test]
fn long_paste_patrol_detects_a_seven_row_attempt_without_text() {
    let text = long_text(60);
    assert_eq!(text.len(), 599);
    let fixture = retain_attempt("codex", &text);
    let rows: Vec<_> = text
        .as_bytes()
        .chunks(90)
        .map(|row| std::str::from_utf8(row).unwrap().trim_end_matches(' '))
        .collect();
    assert_eq!(rows.len(), 7);
    let pane = format!(
        "header\n› {}\n  {}\n\n  status",
        rows[0],
        rows[1..].join("\n  ")
    );
    let captured = serde_json::json!({
        "input": text, "pane": pane, "start": [2, 1],
        "prefix_end": [2 + rows.last().unwrap().len(), rows.len()],
    });
    fs::write(fixture.root().join("captured-layout"), captured.to_string()).unwrap();
    let tmux_model = fixture.root().join("tmux-model");
    let script = fs::read_to_string(&tmux_model).unwrap();
    let capture = "if command == 'has-session':";
    assert!(script.contains(capture));
    fs::write(
        &tmux_model,
        script.replace(capture, "if command == 'list-windows': print('modeled-guarded\\t0'); sys.exit(0)\nif command == 'has-session':"),
    ).unwrap();
    let before = fixture.calls().len();
    let status = super::attempt::status(&fixture, KEY);
    let row = list_row(&fixture, &text);
    assert_eq!(row["status"], "running");
    assert_eq!(
        [
            status["unsubmitted_attempt"].clone(),
            row["unsubmitted_attempt"].clone()
        ],
        [serde_json::json!(true), serde_json::json!(true)]
    );
    assert_eq!(status["composer"], "occupied");
    assert!(
        fixture.calls()[before..]
            .iter()
            .all(|call| call[0] != "send-keys")
    );
    fixture.refused(
        &super::attempt::query(&fixture, "recover", KEY),
        "attempt-recovery-unproven",
    );
    assert_eq!(enter_count(&fixture), 0);

    // Occupancy remains observable when attached. Exact recovery still
    // requires the original detached pane stamp.
    let script = fs::read_to_string(&tmux_model).unwrap();
    let detached = "print('0|0|$42|%42|42|0|0')";
    assert!(script.contains(detached));
    fs::write(
        tmux_model,
        script.replace(detached, "print('1|0|$42|%42|42|0|0')"),
    )
    .unwrap();
    assert_eq!(list_row(&fixture, &text)["unsubmitted_attempt"], true);
    fixture.refused(
        &attempt(&fixture, "recover", &text),
        "attempt-recovery-unproven",
    );
    assert_eq!(enter_count(&fixture), 0);

    fs::write(fixture.root().join("composer"), "").unwrap();
    assert_eq!(
        super::attempt::status(&fixture, KEY)["unsubmitted_attempt"],
        false
    );
    assert_ne!(list_row(&fixture, &text)["unsubmitted_attempt"], true);

    fs::remove_file(fixture.root().join("start-after-paste")).unwrap();
    let later = fixture.send_text(
        Some("later attempt"),
        &[
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "later-key",
        ],
    );
    assert_eq!(later.code, 0, "{}", later.stdout_text());
    fs::write(fixture.root().join("composer"), &text).unwrap();
    assert_ne!(list_row(&fixture, &text)["unsubmitted_attempt"], true);
    assert_eq!(
        super::attempt::status(&fixture, KEY)["unsubmitted_attempt"],
        false
    );
}
