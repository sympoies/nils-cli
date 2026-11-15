//! Guarded delivery against a synthetic Codex OSC-8 footer.
use super::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

// Synthetic styled capture; a second NBSP is owner input, not layout.
fn styled_claude(fixture: &Fixture, footer: &str, post_paste_footer: &str) {
    let path = fixture.root().join("tmux-model");
    let script = fs::read_to_string(&path).unwrap().replace(
        "    suggestion = (root / 'suggestion')",
        "    footer = (root / ('post-paste-footer' if (root / 'paste-completed').exists() else 'styled-footer')).read_text()\n    suggestion = (root / 'suggestion')",
    ).replace(
        "    live = (root / 'live-layout')",
        "    if '-e' in args: top = '\\x1b[38;5;244m' + top + '\\x1b[39m'\n    live = (root / 'live-layout')",
    ).replace("pad(shown)", "pad(shown).ljust(76)");
    fs::write(path, script).unwrap();
    fs::write(fixture.root().join("bare-composer"), "").unwrap();
    fs::write(fixture.root().join("styled-footer"), footer).unwrap();
    fs::write(fixture.root().join("post-paste-footer"), post_paste_footer).unwrap();
}

const STYLED_STATUS: &str = concat!(
    "\u{1b}[39m  \u{1b}[36mModel high ◑ ready\u{1b}[38;5;246m │ ",
    "\u{1b}[33m\u{1b}]8;id=synthetic;https://example.invalid/\u{1b}\\project\u{1b}[38;5;246m\u{1b}]8;;\u{1b}\\ ",
    "\u{1b}[35mbranch\u{1b}[36m ↓0\u{1b}[35m+0\u{1b}[38;5;246m │ ",
    "\u{1b}[2m⏱️  10s ago\u{1b}[0m\u{1b}[38;5;246m │ \u{1b}[2mcontext left…\n",
    "\u{1b}[0m  \u{1b}[2mcontext\u{1b}[0m\u{1b}[38;5;246m ",
    "\u{1b}[32m██\u{1b}[2m░░░░░░░░\u{1b}[0m\u{1b}[38;5;246m │ ",
    "\u{1b}[2msession budget\u{1b}[0m\u{1b}[38;5;210m █\u{1b}[2m░░░░░░░░░\u{1b}[0m\u{1b}[39m   \n",
    "  \u{1b}[38;5;210m⏵⏵ synthetic permissions on\u{1b}[38;5;246m (shift+tab to cycle)\u{1b}[39m                               \n",
    "                                                                 \u{1b}[38;5;246msystem status"
);

#[test]
fn styled_nbsp_empty_composer_observes_the_exact_paste_and_preserves_drafts() {
    for draft in ["", "\u{a0}", "owner draft"] {
        let fixture = Fixture::new("claude");
        styled_claude(&fixture, STYLED_STATUS, STYLED_STATUS);
        fs::write(fixture.root().join("composer"), draft).unwrap();
        // Capture the current synthetic composer without starting the pane model
        // inside the observation budget; other tmux operations keep the model.
        let tmux = fixture.root().join("tmux");
        fs::rename(&tmux, fixture.root().join("tmux-fast")).unwrap();
        fs::write(
            &tmux,
            "#!/bin/sh\nroot=${0%/*}\nif [ \"$1\" = capture-pane ]; then\n    IFS= read -r composer < \"$root/composer\" || :\n    styled=false\n    for arg in \"$@\"; do [ \"$arg\" = -e ] && styled=true; done\n    if [ \"$styled\" = true ]; then\n        printf '\\033[38;5;244m────────────────────────────\\033[39m\\n❯ %-76s\\n────────────────────────────\\n  ' \"$composer\"\n        while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; done < \"$root/styled-footer\"\n    else\n        printf '────────────────────────────\\n❯ %s\\n────────────────────────────\\n  status\\n' \"$composer\"\n    fi\n    for arg in \"$@\"; do\n        if [ \"$arg\" = display-message ]; then\n            cursor=${#composer}\n            if [ -e \"$root/cursor\" ]; then IFS= read -r cursor < \"$root/cursor\" || :; fi\n            printf '%s 1\\n' \"$((cursor + 2))\"\n        fi\n    done\n    exit 0\nfi\nexec \"$root/tmux-fast\" \"$@\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        let output = fixture.send_text(Some("read mailbox"), &["--require-empty-composer"]);
        if draft.is_empty() {
            assert_eq!(output.code, 0, "{}", output.stdout_text());
            assert_eq!(
                output.stdout_json()["data"]["submission"]["outcome"],
                "submitted"
            );
            assert_eq!(
                fs::read_to_string(fixture.root().join("submissions")).unwrap(),
                "\"read mailbox\"\n"
            );
        } else {
            fixture.refused(&output, "send-composer-not-empty");
            fixture.no_delivery();
            assert_eq!(
                fs::read_to_string(fixture.root().join("composer")).unwrap(),
                draft
            );
        }
    }
}

#[test]
fn styled_composer_refusals_name_the_stage_and_observation_without_pane_text() {
    for after_paste in [false, true] {
        let fixture = Fixture::new("claude");
        let unsupported = "\u{1b}]52;;private-fixture-marker\u{7}";
        styled_claude(
            &fixture,
            if after_paste {
                STYLED_STATUS
            } else {
                unsupported
            },
            unsupported,
        );
        // Return the malformed styled capture directly, without starting the
        // Python pane model inside the short observation deadline.
        let tmux = fixture.root().join("tmux");
        fs::write(
            fixture.root().join("styled-admission-capture"),
            format!(
                "\u{1b}[38;5;244m────────────────────────────\u{1b}[39m\n❯\u{a0}{}\n────────────────────────────\n  {STYLED_STATUS}\n",
                " ".repeat(76)
            ),
        )
        .unwrap();
        fs::rename(&tmux, fixture.root().join("tmux-fast")).unwrap();
        fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nroot=${{0%/*}}\nif [ \"$1\" = capture-pane ]; then\n    for arg in \"$@\"; do\n        if [ \"$arg\" = -e ]; then\n            if [ {} = false ] || [ -e \"$root/paste-completed\" ]; then\n                printf '\\033]52;;private-fixture-marker\\007'\n            else\n                cat \"$root/styled-admission-capture\"\n            fi\n            exit 0\n        fi\n    done\nfi\nexec \"$root/tmux-fast\" \"$@\"\n",
                after_paste
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        let output = fixture.send_text(Some("read mailbox"), &["--require-empty-composer"]);
        assert_eq!(output.code, 1, "{}", output.stderr_text());
        let value = output.stdout_json();
        let details = &value["error"]["details"];
        assert_eq!(
            details["stage"],
            if after_paste { "submit" } else { "admission" }
        );
        assert_eq!(details["pasted"], after_paste);
        assert_eq!(details["submitted"], false);
        assert!(!output.stdout_text().contains("private-fixture-marker"));
        assert!(!fixture.root().join("submissions").exists());
        if !after_paste {
            fixture.no_delivery();
        }
    }
}

#[test]
fn codex_hyperlink_footer_submits_once_and_rejects_the_same_key() {
    let provider = "codex";
    let fixture = Fixture::new(provider);
    fs::write(fixture.root().join("native-layout"), "").unwrap();
    let guards = [
        "--expect-runtime",
        "runtime-one",
        "--expect-provider-session",
        "provider-one",
        "--require-idle",
        "--require-empty-composer",
        "--idempotency-key",
        "native-empty",
    ];
    let output = fixture.send_text(Some("continue-fixture  "), &guards);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
    let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
    assert_eq!(submissions.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<String>(submissions.trim()).unwrap(),
        "continue-fixture  "
    );
    fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions")).unwrap(),
        submissions
    );
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|c| c[0] == "paste-buffer")
            .count(),
        1
    );
}

#[test]
fn codex_hyperlink_footer_preserves_drafts_and_load_time_conflicts() {
    let provider = "codex";
    for draft in ["user draft", " ", "first\n  second"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("native-layout"), "").unwrap();
        fs::write(fixture.root().join("composer"), draft).unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        fixture.refused(
            &output,
            if draft.contains('\n') {
                "send-composer-unobservable"
            } else {
                "send-composer-not-empty"
            },
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            draft
        );
    }
    for (marker, reason) in [
        ("draft-on-load", "send-composer-not-empty"),
        ("start-on-load", "send-session-not-idle"),
    ] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("native-layout"), "").unwrap();
        fs::write(fixture.root().join(marker), "").unwrap();
        fixture.refused(
            &fixture.send(&["--require-idle", "--require-empty-composer"]),
            reason,
        );
        fixture.no_delivery();
    }
}

#[test]
fn synthetic_dim_suggestion_sends_once_and_identical_human_drafts_block() {
    for provider in ["claude", "codex"] {
        let suggestion = "Continue reviewing the pending changes";
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("suggestion"), suggestion).unwrap();
        let guards = [
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "dim-suggestion",
        ];
        let output = fixture.send_text(Some("Proceed with the synthetic check"), &guards);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["outcome"],
            "submitted"
        );
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");

        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), suggestion).unwrap();
        fixture.refused(&fixture.send(&guards), "send-composer-not-empty");
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            suggestion
        );
    }
}

#[test]
fn claude_bare_nbsp_composer_admits_one_guarded_submission() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("bare-composer"), "").unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn claude_hyperlink_footer_admits_empty_hints_and_preserves_plain_drafts() {
    let suggestion = "Continue the fixture check";
    let padding = " ".repeat(52);
    for dim_suggestion in [false, true] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("native-layout"), "").unwrap();
        if dim_suggestion {
            fs::write(fixture.root().join("suggestion"), suggestion).unwrap();
            fs::write(fixture.root().join("suggestion-padding"), &padding).unwrap();
        } else {
            fs::write(fixture.root().join("bare-composer"), "").unwrap();
        }
        let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
    let fixture = Fixture::new("claude");
    let draft = format!("{suggestion}{padding}");
    fs::write(fixture.root().join("native-layout"), "").unwrap();
    fs::write(fixture.root().join("composer"), &draft).unwrap();
    fixture.refused(
        &fixture.send(&["--require-empty-composer"]),
        "send-composer-not-empty",
    );
    fixture.no_delivery();
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        draft
    );
}

#[test]
fn claude_per_word_dim_suggestion_admits_one_guarded_submission() {
    let suggestion = "Read your mailbox and proceed";
    let guards = ["--require-idle", "--require-empty-composer"];
    for titled in [false, true] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("suggestion"), suggestion).unwrap();
        fs::write(fixture.root().join("suggestion-words"), "").unwrap();
        if titled {
            fs::write(fixture.root().join("titled-border"), "").unwrap();
        }
        let output = fixture.send(&guards);
        assert_eq!(output.code, 0, "titled {titled}: {}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[test]
fn claude_typed_draft_with_the_suggestion_words_is_still_refused() {
    let suggestion = "Read your mailbox and proceed";
    for titled in [false, true] {
        let fixture = Fixture::new("claude");
        fs::write(fixture.root().join("composer"), suggestion).unwrap();
        if titled {
            fs::write(fixture.root().join("titled-border"), "").unwrap();
        }
        fixture.refused(
            &fixture.send(&["--require-idle", "--require-empty-composer"]),
            "send-composer-not-empty",
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            suggestion
        );
    }
}
