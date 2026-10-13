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
            None
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
        assert_eq!(
            exact_composer_input("claude", &claude("  café  ", "", &footer)),
            None
        );
    }
}

#[test]
fn blank_row_compatibility_never_erases_a_space_or_multiline_draft() {
    for input in [" ", "user draft "] {
        assert_eq!(
            exact_composer_input("claude", &claude(input, "", "status")).as_deref(),
            Some(input)
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
