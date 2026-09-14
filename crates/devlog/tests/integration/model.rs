//! Unit-level coverage for parsing and rendering, which own the conventions
//! that were previously enforced by nothing.

use nils_devlog::entry::Entry;
use nils_devlog::index::{listed_months, render_months};
use nils_devlog::model::{EntryDate, Month};
use pretty_assertions::assert_eq;

#[test]
fn month_parses_and_renders_zero_padded() {
    let month: Month = "2026-04".parse().expect("valid month");
    assert_eq!(month.to_string(), "2026-04");
    assert_eq!(month.heading(), "# Development log - 2026-04");
}

#[test]
fn month_rejects_malformed_values() {
    for value in [
        "2026-13", "2026-00", "26-04", "2026-4", "2026/04", "", "abcd-ef",
    ] {
        assert!(
            value.parse::<Month>().is_err(),
            "expected '{value}' to be rejected"
        );
    }
}

#[test]
fn entry_date_rejects_days_outside_the_month() {
    assert!("2026-02-30".parse::<EntryDate>().is_err());
    assert!("2026-04-31".parse::<EntryDate>().is_err());
    assert!("2026-04-00".parse::<EntryDate>().is_err());
    assert!("2026-04-30".parse::<EntryDate>().is_ok());
}

#[test]
fn entry_date_honors_leap_years() {
    // 2024 is a leap year, 2100 is not despite being divisible by four.
    assert!("2024-02-29".parse::<EntryDate>().is_ok());
    assert!("2100-02-29".parse::<EntryDate>().is_err());
    assert!("2000-02-29".parse::<EntryDate>().is_ok());
}

#[test]
fn entry_renders_sections_as_headings_not_bold_labels() {
    // MD036 is enabled in this workspace's lint baseline, so a bold label
    // would fail the docs lane. This assertion is the contract.
    let entry = Entry {
        title: "Did a thing".to_string(),
        result: vec!["Shipped it".to_string()],
        why: vec!["It was needed".to_string()],
        evidence: vec!["Ran the gate".to_string()],
        links: vec!["`abc12345`".to_string()],
        ..Entry::default()
    };
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let rendered = entry.render(date);

    assert!(rendered.starts_with("## 2026-04-17 - Did a thing\n"));
    assert!(rendered.contains("### Result\n"));
    assert!(rendered.contains("### Why / context\n"));
    assert!(rendered.contains("### Evidence\n"));
    assert!(rendered.contains("### Links\n"));
    assert!(!rendered.contains("**Result**"));
}

#[test]
fn entry_omits_follow_ups_when_empty_and_renders_it_when_present() {
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let base = Entry {
        title: "Title".to_string(),
        result: vec!["r".to_string()],
        ..Entry::default()
    };
    assert!(!base.render(date).contains("### Follow-ups"));

    let with_follow_up = Entry {
        follow_ups: vec!["later".to_string()],
        ..base
    };
    assert!(with_follow_up.render(date).contains("### Follow-ups"));
}

#[test]
fn index_months_round_trip() {
    let index =
        "# Development log\n\n## Months\n\n- [2026-05](2026-05.md)\n- [2026-04](2026-04.md)\n";
    let months = listed_months(index);
    assert_eq!(
        months.iter().map(Month::to_string).collect::<Vec<_>>(),
        vec!["2026-05", "2026-04"]
    );
    assert_eq!(
        render_months(&months),
        "- [2026-05](2026-05.md)\n- [2026-04](2026-04.md)"
    );
}

#[test]
fn index_months_ignores_links_outside_the_months_section() {
    let index =
        "## Conventions\n\n- [2026-01](2026-01.md)\n\n## Months\n\n- [2026-05](2026-05.md)\n";
    let months = listed_months(index);
    assert_eq!(
        months.iter().map(Month::to_string).collect::<Vec<_>>(),
        vec!["2026-05"]
    );
}

#[test]
fn render_months_sorts_newest_first() {
    let months: Vec<Month> = ["2026-01", "2026-12", "2026-06"]
        .iter()
        .map(|value| value.parse().expect("valid month"))
        .collect();
    assert_eq!(
        render_months(&months),
        "- [2026-12](2026-12.md)\n- [2026-06](2026-06.md)\n- [2026-01](2026-01.md)"
    );
}

#[test]
fn bullets_wrap_so_generated_entries_pass_the_markdown_line_limit() {
    // Regression: the first implementation emitted each bullet on one line,
    // so a normal-length bullet produced a 200+ character line and failed
    // MD013 in the workspace lint baseline. A generator whose output fails
    // the repository's own docs lane is worse than writing entries by hand.
    let long = "Enabled the development log and backfilled eighty-eight entries \
                covering January through September, written from the full commit \
                history on the default branch and weighted by milestone rather \
                than by month.";
    let entry = Entry {
        title: "Long bullet".to_string(),
        result: vec![long.to_string()],
        why: vec!["short".to_string()],
        evidence: vec!["short".to_string()],
        links: vec!["short".to_string()],
        ..Entry::default()
    };
    let date: EntryDate = "2026-09-14".parse().expect("valid date");
    let rendered = entry.render(date);

    for line in rendered.lines() {
        assert!(
            line.chars().count() <= 140,
            "line exceeds the MD013 limit: {line}"
        );
    }
    // Continuation lines align under the bullet text.
    assert!(
        rendered.contains("\n  "),
        "expected wrapped continuation lines"
    );
}

#[test]
fn a_single_unbreakable_token_is_emitted_intact() {
    // A long URL has no whitespace to wrap at. Emitting it intact and letting
    // it overrun is correct; breaking it would produce a link that no longer
    // resolves.
    let url = format!("https://example.com/{}", "x".repeat(120));
    let entry = Entry {
        title: "Link".to_string(),
        result: vec![url.clone()],
        ..Entry::default()
    };
    let date: EntryDate = "2026-09-14".parse().expect("valid date");
    let rendered = entry.render(date);
    assert!(rendered.contains(&url), "the token must not be broken");
}
