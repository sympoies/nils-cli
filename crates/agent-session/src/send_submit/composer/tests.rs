//! Synthetic capture-shaped regressions; no live provider payloads.
use super::exact_composer_input;
use pretty_assertions::assert_eq;

fn claude(input: &str, rows: &str, footer: &str) -> String {
    format!("────────────────────\n❯\u{a0}{input}\n{rows}────────────────────\n  {footer}")
}

fn codex(input: &str, rows: &str, footer: &str) -> String {
    format!("› {input}\n{rows}\n  {footer}")
}

#[test]
fn native_blank_rows_after_a_dim_hint_are_empty() {
    for rows in ["", "\n", "\n\n"] {
        assert_eq!(
            exact_composer_input(
                "claude",
                &claude("\u{1b}[2mTry \"edit a file\"\u{1b}[22m", rows, "status")
            ),
            Some(String::new())
        );
        assert_eq!(
            exact_composer_input(
                "codex",
                &codex(
                    "\u{1b}[2mAsk Codex to do anything\u{1b}[22m",
                    rows,
                    "status"
                )
            ),
            Some(String::new())
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
        for (provider, pane) in [
            ("claude", claude("  café  ", "", &footer)),
            ("codex", codex("  café  ", "", &footer)),
        ] {
            assert_eq!(
                exact_composer_input(provider, &pane).as_deref(),
                Some("  café  ")
            );
        }
    }
}

#[test]
fn blank_row_compatibility_never_erases_a_space_or_multiline_draft() {
    for input in [" ", "user draft "] {
        assert_eq!(
            exact_composer_input("claude", &claude(input, "\n", "status")).as_deref(),
            Some(input)
        );
        assert_eq!(
            exact_composer_input("codex", &codex(input, "\n", "status")).as_deref(),
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
