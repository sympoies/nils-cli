//! Qualified generic captures and separately labeled synthetic OSC-8 variants.
use super::exact_composer_input;
use pretty_assertions::assert_eq;

fn claude(input: &str, rows: &str, footer: &str) -> String {
    format!("────────────────────\n❯\u{a0}{input}\n{rows}────────────────────\n  {footer}")
}

fn codex(input: &str, rows: &str, footer: &str) -> String {
    format!("› {input}\n{rows}\n  {footer}")
}

#[test]
fn extra_composer_rows_remain_unobservable() {
    for rows in ["\n", "\n\n"] {
        assert_eq!(
            exact_composer_input("claude", &claude("", rows, "status")),
            None
        );
        assert_eq!(
            exact_composer_input("codex", &codex("", rows, "status")),
            None
        );
    }
}

#[test]
fn osc8_hyperlinks_in_a_footer_preserve_exact_input_and_hint_style() {
    for terminator in ["\u{7}", "\u{1b}\\"] {
        let footer = format!(
            "status \u{1b}]8;id=fixture;https://example.invalid/{terminator}link\u{1b}]8;;{terminator}"
        );
        assert_eq!(
            exact_composer_input(
                "claude",
                &claude("\u{1b}[2mTry \"edit a file\"\u{1b}[22m", "", &footer)
            ),
            Some(String::new())
        );
        assert_eq!(
            exact_composer_input(
                "codex",
                &codex("\u{1b}[2mAsk Codex to do anything\u{1b}[22m", "", &footer)
            ),
            Some(String::new())
        );
        assert_eq!(
            exact_composer_input("codex", &codex("  café  ", "", &footer)).as_deref(),
            Some("  café  ")
        );
        // Claude's trailing spaces are indistinguishable from its padding.
        assert_eq!(
            exact_composer_input("claude", &claude("  café  ", "", &footer)).as_deref(),
            Some("  café")
        );
    }
}

#[test]
fn whitespace_and_multiline_drafts_remain_protected() {
    for input in [" ", "user draft "] {
        // Claude's trailing spaces are indistinguishable from its padding.
        assert_eq!(
            exact_composer_input("claude", &claude(input, "", "status")).as_deref(),
            Some(input.trim_end_matches(' '))
        );
        assert_eq!(
            exact_composer_input("codex", &codex(input, "", "status")).as_deref(),
            Some(input)
        );
    }
    for rows in ["  second line\n", " \n", "\n  second line\n"] {
        assert_eq!(
            exact_composer_input("claude", &claude("first", rows, "status")),
            None
        );
        assert_eq!(
            exact_composer_input("codex", &codex("first", rows, "status")),
            None
        );
    }
}

#[test]
fn hyperlinks_never_hide_composer_text_or_replace_dim_proof() {
    let linked = "\u{1b}]8;;https://example.invalid/\u{1b}\\user draft\u{1b}]8;;\u{1b}\\";
    assert_eq!(
        exact_composer_input("codex", &codex(linked, "", "status")).as_deref(),
        Some("user draft")
    );
    assert_eq!(
        exact_composer_input("claude", &claude(linked, "", "status")).as_deref(),
        Some("user draft")
    );
    let unstyled =
        "\u{1b}]8;;https://example.invalid/\u{1b}\\Ask Codex to do anything\u{1b}]8;;\u{1b}\\";
    assert_eq!(
        exact_composer_input("codex", &codex(unstyled, "", "status")),
        None
    );
    let partial = "\u{1b}[2mAsk Codex\u{1b}[22m to do anything";
    assert_eq!(
        exact_composer_input("codex", &codex(partial, "\n", "status")),
        None
    );
}

#[test]
fn malformed_or_screen_mutating_escapes_remain_unobservable() {
    for footer in [
        "\u{1b}]8;;unterminated",
        "\u{1b}]8;;bad\nurl\u{7}",
        "\u{1b}]52;;clipboard\u{7}",
        "\u{1b}[2J",
        "\u{1b}]8;;url\u{1b}X",
        "\u{1b}]8;missing-delimiter\u{7}",
    ] {
        assert_eq!(exact_composer_input("codex", &codex("", "", footer)), None);
        assert_eq!(
            exact_composer_input("claude", &claude("", "", footer)),
            None
        );
    }
}

#[test]
fn qualified_codex_empty_capture_retains_dim_hint_authority() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("codex-native-captures.json")).unwrap();
    for case in data["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["expected_empty"] == true)
    {
        let capture = case["capture"].as_str().unwrap();
        assert_eq!(
            exact_composer_input("codex", capture).as_deref(),
            Some(""),
            "case {}",
            case["case"]
        );
    }
}

#[test]
fn qualified_codex_drafts_never_classify_as_empty() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("codex-native-captures.json")).unwrap();
    for case in data["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["expected_empty"] == false)
    {
        let input = exact_composer_input("codex", case["capture"].as_str().unwrap());
        assert!(
            input.is_none_or(|input| !input.is_empty()),
            "case {}",
            case["case"]
        );
    }
}

#[test]
fn qualified_codex_capture_with_synthetic_footer_hyperlink_is_observable() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("codex-native-captures.json")).unwrap();
    for case in data["fixtures"].as_array().unwrap() {
        let capture = case["capture"].as_str().unwrap();
        // The qualified corpus has SGR only. This separate overlay exercises
        // the reported hyperlink metadata without relabeling it native evidence.
        let start = capture.rfind("\n  \u{1b}[38;2").unwrap() + 1;
        let end = start + capture[start..].find('\n').unwrap();
        for terminator in ["\u{7}", "\u{1b}\\"] {
            let pane = format!(
                "{}\u{1b}]8;;https://example.invalid/{terminator}{}\u{1b}]8;;{terminator}{}",
                &capture[..start],
                &capture[start..end],
                &capture[end..]
            );
            assert_eq!(
                exact_composer_input("codex", &pane),
                exact_composer_input("codex", capture),
                "case {}",
                case["case"]
            );
        }
    }
}

#[test]
fn hyperlink_metadata_has_a_bounded_payload() {
    let boundary = format!("\u{1b}]8;;{}\u{7}status\u{1b}]8;;\u{7}", "x".repeat(4093));
    assert_eq!(
        exact_composer_input("codex", &codex("", "", &boundary)).as_deref(),
        Some("")
    );
    assert_eq!(
        exact_composer_input("claude", &claude("", "", &boundary)).as_deref(),
        Some("")
    );
    let oversized = format!("\u{1b}]8;;{}\u{7}", "x".repeat(4094));
    assert_eq!(
        exact_composer_input("codex", &codex("", "", &oversized)),
        None
    );
    assert_eq!(
        exact_composer_input("claude", &claude("", "", &oversized)),
        None
    );
}

#[test]
fn synthetic_dim_suggestions_are_empty_for_both_harnesses_but_typed_twins_are_drafts() {
    for (provider, render) in [
        ("claude", claude as fn(&str, &str, &str) -> String),
        ("codex", codex),
    ] {
        let suggestion = "Continue reviewing the pending changes";
        let dim = format!("\u{1b}[2m{suggestion}\u{1b}[22m");
        assert_eq!(
            exact_composer_input(provider, &render(&dim, "", "status")).as_deref(),
            Some("")
        );
        assert_eq!(
            exact_composer_input(provider, &render(suggestion, "", "status")).as_deref(),
            Some(suggestion)
        );
        let partial = "\u{1b}[2mContinue\u{1b}[22m reviewing the pending changes";
        assert!(
            exact_composer_input(provider, &render(partial, "", "status"))
                .is_none_or(|input| !input.is_empty())
        );
        let whitespace = "\u{1b}[2m \u{1b}[22m";
        assert_eq!(
            exact_composer_input(provider, &render(whitespace, "", "status")).as_deref(),
            Some(" ")
        );
    }
}

#[test]
fn claude_nbsp_separator_preserves_empty_suggestion_and_draft_states() {
    for (input, expected) in [
        ("", ""),
        (
            "\u{1b}[2mContinue reviewing the pending changes\u{1b}[22m",
            "",
        ),
        (
            "Continue reviewing the pending changes",
            "Continue reviewing the pending changes",
        ),
    ] {
        assert_eq!(
            exact_composer_input(
                "claude",
                &claude(input, "", "Update installed; restart to apply")
            )
            .as_deref(),
            Some(expected),
            "input {input:?}"
        );
    }
}

#[test]
fn claude_nonbreaking_separators_preserve_empty_suggestion_and_draft_states() {
    for separator in ['\u{2007}', '\u{202f}'] {
        for (input, expected) in [
            ("", ""),
            (
                "\u{1b}[2mContinue reviewing the pending changes\u{1b}[22m",
                "",
            ),
            (
                "Continue reviewing the pending changes",
                "Continue reviewing the pending changes",
            ),
        ] {
            let pane = claude(input, "", "status").replacen('\u{a0}', &separator.to_string(), 1);
            assert_eq!(
                exact_composer_input("claude", &pane).as_deref(),
                Some(expected),
                "separator {separator:?}, input {input:?}"
            );
        }
    }
}

#[test]
fn nonbreaking_separators_never_hide_choices_or_additional_draft_characters() {
    for separator in [' ', '\u{a0}', '\u{2007}', '\u{202f}'] {
        let choice = claude("\u{1b}[2m1. Confirm\u{1b}[22m", "", "status").replacen(
            '\u{a0}',
            &separator.to_string(),
            1,
        );
        assert_eq!(exact_composer_input("claude", &choice), None);
        for draft in [
            "\u{a0}", "\u{2007}", "\u{202f}", "\u{200b}", "\u{feff}", " café",
        ] {
            let pane = claude(draft, "", "status").replacen('\u{a0}', &separator.to_string(), 1);
            assert_eq!(
                exact_composer_input("claude", &pane).as_deref(),
                Some(draft)
            );
        }
    }
}

fn claude_fullpane_fixture() -> (String, String, usize) {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("claude-native-capture.json")).unwrap();
    (
        data["capture"].as_str().unwrap().to_string(),
        data["suggestion"].as_str().unwrap().to_string(),
        data["padding_spaces"].as_u64().unwrap() as usize,
    )
}

#[test]
fn claude_fullpane_nbsp_dim_suggestion_is_empty() {
    let (pane, _, _) = claude_fullpane_fixture();
    assert_eq!(exact_composer_input("claude", &pane).as_deref(), Some(""));
}

#[test]
fn claude_fullpane_nbsp_plain_text_remains_a_draft_without_padding() {
    let (pane, suggestion, padding) = claude_fullpane_fixture();
    // Synthetic plain twin: preserve the layout, caption and occupied spaces.
    let pane = pane.replace(&format!("\u{1b}[2m{suggestion}\u{1b}[0m"), &suggestion);
    assert!(padding > 0);
    assert_eq!(
        exact_composer_input("claude", &pane).as_deref(),
        Some(suggestion.as_str())
    );
}

#[test]
fn claude_fullpane_bare_nbsp_is_empty() {
    let (pane, suggestion, padding) = claude_fullpane_fixture();
    // Synthetic bare overlay: remove the suggestion and its renderer padding.
    let pane = pane.replace(
        &format!("\u{1b}[2m{suggestion}\u{1b}[0m{}", " ".repeat(padding)),
        "",
    );
    assert_eq!(exact_composer_input("claude", &pane).as_deref(), Some(""));
}

#[test]
fn dim_suggestion_padding_does_not_hide_real_text_or_whitespace_drafts() {
    let padded = "\u{1b}[2mContinue the fixture check\u{1b}[0m    ";
    assert_eq!(
        exact_composer_input("claude", &claude(padded, "", "status")).as_deref(),
        Some("")
    );
    for input in [
        "\u{1b}[2mContinue\u{1b}[0m the fixture check    ",
        "\u{1b}[2mContinue the fixture check\u{1b}[0m user draft ",
        "\u{1b}[2mContinue the fixture check\u{1b}[0m\u{a0}",
        "\u{1b}[2mContinue the fixture check\u{1b}[0m\u{200b}",
        "\u{1b}[2m \u{1b}[0m    ",
    ] {
        assert!(
            exact_composer_input("claude", &claude(input, "", "status"))
                .is_none_or(|draft| !draft.is_empty()),
            "input {input:?}"
        );
    }
}

/// Claude draws a suggestion as one dim run per word; the spaces between the
/// words carry no style. This is the layout of a live idle pane.
fn dim_words(suggestion: &str) -> String {
    suggestion
        .split(' ')
        .map(|word| format!("\u{1b}[2m{word}\u{1b}[0m"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn per_word_dim_suggestion_is_empty_but_typed_twins_stay_drafts() {
    let suggestion = "Read your mailbox and proceed";
    let padding = " ".repeat(20);
    for provider in ["claude", "codex"] {
        let render = if provider == "claude" { claude } else { codex };
        let styled = dim_words(suggestion);
        assert_eq!(
            exact_composer_input(provider, &render(&styled, "", "status")).as_deref(),
            Some(""),
            "{provider}"
        );
        // Typed text is never dim, whatever its words.
        assert_eq!(
            exact_composer_input(provider, &render(suggestion, "", "status")).as_deref(),
            Some(suggestion),
            "{provider}"
        );
    }
    assert_eq!(
        exact_composer_input(
            "claude",
            &claude(&format!("{}{padding}", dim_words(suggestion)), "", "status")
        )
        .as_deref(),
        Some("")
    );
}

#[test]
fn per_word_dim_suggestion_never_hides_a_partly_typed_or_padded_draft() {
    let words = dim_words("Read your mailbox and proceed");
    for input in [
        // One typed word among dim words.
        format!("{} typed", words),
        format!("typed {words}"),
        words.replacen("\u{1b}[2mmailbox\u{1b}[0m", "mailbox", 1),
        // Unstyled whitespace around or instead of the suggestion.
        format!(" {words}"),
        format!("{words}\u{a0}"),
        "\u{1b}[2mRead\u{1b}[0m\u{a0}\u{1b}[2mmailbox\u{1b}[0m".to_string(),
        "\u{a0}".to_string(),
        "\u{1b}[2m \u{1b}[0m \u{1b}[2m \u{1b}[0m".to_string(),
    ] {
        for provider in ["claude", "codex"] {
            let render = if provider == "claude" { claude } else { codex };
            assert!(
                exact_composer_input(provider, &render(&input, "", "status"))
                    .is_none_or(|draft| !draft.is_empty()),
                "{provider} {input:?}"
            );
        }
    }
}

#[test]
fn claude_titled_top_border_still_locates_the_composer() {
    // A long session title leaves only a few border dashes on the top rule.
    let titled = "─── a long session title that fills the width of the pane ─";
    let rule = "────────────────────";
    for (input, expected) in [("", ""), ("user draft", "user draft")] {
        let pane = format!("{titled}\n❯\u{a0}{input}\n{rule}\n  status");
        assert_eq!(
            exact_composer_input("claude", &pane).as_deref(),
            Some(expected)
        );
    }
    let pane = format!(
        "{titled}\n❯\u{a0}{}\n{rule}\n  status",
        dim_words("Try it now")
    );
    assert_eq!(exact_composer_input("claude", &pane).as_deref(), Some(""));
    // Prose that merely starts with dashes is not a composer border.
    for line in ["─── not a border", "── a ──", "─── x ─ y"] {
        let pane = format!("{line}\n❯\u{a0}\n{rule}\n  status");
        assert_eq!(exact_composer_input("claude", &pane), None, "{line}");
    }
}

/// Claude draws the title into the top border and gives it every dash that is
/// left; a long title leaves none, so the line starts with the title itself.
#[test]
fn claude_title_without_border_dashes_still_locates_the_composer() {
    let rule = "────────────────────";
    for titled in [
        " a long session title that fills the pane and is cut off… ─",
        " title ─",
        "── title ─",
    ] {
        for (input, expected) in [("", ""), ("user draft", "user draft")] {
            let pane = format!("{titled}\n❯\u{a0}{input}\n{rule}\n  status");
            assert_eq!(
                exact_composer_input("claude", &pane).as_deref(),
                Some(expected),
                "{titled}"
            );
        }
    }
    // Indented prose is a border only directly above the input row.
    let pane = format!(" prose that ends in a dash ─\nnot input\n{rule}\n  status");
    assert_eq!(exact_composer_input("claude", &pane), None);
    let pane = format!(" prose that ends in a dash\n❯\u{a0}\n{rule}\n  status");
    assert_eq!(exact_composer_input("claude", &pane), None);
}

/// Claude writes spaces after the input up to the pane width, so the capture
/// of an empty or short input carries them whatever the capture flags say.
#[test]
fn claude_renderer_padding_is_not_input() {
    let padding = " ".repeat(60);
    for (input, expected) in [
        ("", Some("")),
        ("draft one two", Some("draft one two")),
        ("\u{1b}[2mContinue\u{1b}[0m", Some("")),
    ] {
        assert_eq!(
            exact_composer_input(
                "claude",
                &claude(&format!("{input}{padding}"), "", "status")
            )
            .as_deref(),
            expected,
            "{input:?}"
        );
    }
    // Only unstyled ASCII spaces are padding: other blanks stay input.
    for input in [
        format!("\u{a0}{padding}"),
        format!("\u{200b}{padding}"),
        format!("\u{1b}[2m \u{1b}[0m{padding}"),
        format!("\u{3000}{padding}"),
    ] {
        assert!(
            exact_composer_input("claude", &claude(&input, "", "status"))
                .is_none_or(|draft| !draft.is_empty()),
            "{input:?}"
        );
    }
    // Codex pads nothing, so its occupied spaces stay exact.
    assert_eq!(
        exact_composer_input("codex", &codex("draft  ", "", "status")).as_deref(),
        Some("draft  ")
    );
}

#[test]
fn composer_holds_pasted_text_modulo_claude_padding() {
    use super::composer_holds;
    assert!(composer_holds("claude", "reply now", "reply now"));
    assert!(composer_holds("claude", "reply now", "reply now  "));
    assert!(!composer_holds("claude", "reply now and more", "reply now"));
    assert!(!composer_holds("claude", "", "reply now"));
    assert!(!composer_holds("claude", "reply now", ""));
    assert!(composer_holds("codex", "reply now", "reply now"));
    assert!(!composer_holds("codex", "reply now", "reply now  "));
}
