//! A Claude input row on the first row of the capture has no rule above it in
//! view. Its closing rule below still bounds the box, so the row is read as
//! usual; a draft, a choice or a multi-row input stays refused.
use super::{Unobserved, direct_composer_observation, exact_composer_input};
use pretty_assertions::assert_eq;

const RULE: &str = "────────────────────";
const FOOTER: &str = "  synthetic footer";
const HINT: &str = "\u{1b}[2mTry\u{1b}[0m \u{1b}[2m\"synthetic hint\"\u{1b}[0m";

fn first_row(rows: &str) -> String {
    format!("{rows}\n{RULE}\n{FOOTER}\n\n")
}

#[test]
fn first_row_input_with_a_dim_suggestion_is_empty() {
    let pane = first_row(&format!("❯\u{a0}{HINT}      "));
    assert_eq!(exact_composer_input("claude", &pane).as_deref(), Some(""));
    assert_eq!(
        direct_composer_observation("claude", &pane),
        Ok(String::new())
    );
}

#[test]
fn first_row_draft_is_read_as_a_draft_and_never_as_empty() {
    let pane = first_row("❯\u{a0}synthetic draft");
    assert_eq!(
        exact_composer_input("claude", &pane).as_deref(),
        Some("synthetic draft")
    );
    assert_eq!(
        direct_composer_observation("claude", &pane),
        Ok("synthetic draft".to_string())
    );
}

#[test]
fn first_row_hint_look_alike_in_plain_text_is_a_draft() {
    let pane = first_row("❯\u{a0}Try \"synthetic hint\"");
    assert_eq!(exact_composer_input("claude", &pane), None);
}

#[test]
fn first_row_without_one_closing_rule_stays_unlocated() {
    for pane in [
        "❯\u{a0}synthetic draft\n  synthetic footer\n".to_string(),
        format!("❯\u{a0}one\n❯\u{a0}two\n{RULE}\n{FOOTER}\n"),
        format!("{RULE}\n{FOOTER}\n"),
        format!("plain row\n{RULE}\n{FOOTER}\n"),
    ] {
        assert_eq!(exact_composer_input("claude", &pane), None, "{pane:?}");
    }
    assert_eq!(
        direct_composer_observation("claude", "❯\u{a0}synthetic draft\n  synthetic footer\n"),
        Err(Unobserved::ComposerNotLocated)
    );
}

#[test]
fn an_input_row_that_is_not_the_first_row_stays_unlocated_without_its_opening_rule() {
    for pane in [
        // A transcript row above, then the input row and a closing rule.
        format!("synthetic transcript row\n❯\u{a0}{HINT}\n{RULE}\n{FOOTER}\n"),
        // A transcript echo of an earlier prompt above the closing rule.
        format!("synthetic transcript row\n❯\u{a0}earlier prompt\n{RULE}\n{FOOTER}\n"),
        // A blank row above the input row.
        format!("\n❯\u{a0}{HINT}\n{RULE}\n{FOOTER}\n"),
    ] {
        assert_eq!(exact_composer_input("claude", &pane), None, "{pane:?}");
        assert_eq!(
            direct_composer_observation("claude", &pane),
            Err(Unobserved::ComposerNotLocated),
            "{pane:?}"
        );
    }
}

#[test]
fn a_multi_line_draft_whose_first_row_is_out_of_view_stays_refused() {
    for pane in [
        format!("  second row of a draft\n{RULE}\n{FOOTER}\n"),
        format!("  second row\n  third row\n{RULE}\n{FOOTER}\n"),
    ] {
        assert_eq!(exact_composer_input("claude", &pane), None, "{pane:?}");
        assert_eq!(
            direct_composer_observation("claude", &pane),
            Err(Unobserved::ComposerNotLocated),
            "{pane:?}"
        );
    }
}

#[test]
fn first_row_input_with_a_list_below_its_rule_stays_unlocated() {
    // A command autocomplete or picker list below the rule is not a status
    // footer: an Enter would answer the list, so the row is not read.
    for pane in [
        format!("❯\u{a0}/synthetic\n{RULE}\n  ❯ /synthetic one\n    /synthetic two\n"),
        format!("❯\u{a0}{HINT}\n{RULE}\n  ❯ synthetic choice\n"),
        format!("❯\u{a0}synthetic draft\n{RULE}\n  ❯ synthetic choice\n"),
    ] {
        assert_eq!(exact_composer_input("claude", &pane), None, "{pane:?}");
        assert_eq!(
            direct_composer_observation("claude", &pane),
            Err(Unobserved::ComposerNotLocated),
            "{pane:?}"
        );
    }
}

#[test]
fn first_row_choice_multi_row_input_and_dialog_stay_refused() {
    for pane in [
        first_row("❯ 1. Yes"),
        format!("❯\u{a0}one\n  two\n{RULE}\n{FOOTER}\n"),
        format!("❯\u{a0}{HINT}\n{RULE}\n  enter to confirm · esc to cancel\n"),
    ] {
        assert_eq!(exact_composer_input("claude", &pane), None, "{pane:?}");
    }
}
