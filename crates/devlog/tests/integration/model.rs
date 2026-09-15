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

#[test]
fn a_conflict_marker_is_found_at_its_one_based_line() {
    let contents = "# Development log - 2026-04\n\n<<<<<<< HEAD\n## 2026-04-20 - Ours\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), Some(3));
}

#[test]
fn the_closing_marker_alone_is_still_a_conflict() {
    // A partially hand-resolved file keeps only some markers; any one of them
    // means the merge is unfinished.
    let contents = "# Development log - 2026-04\n>>>>>>> feature\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), Some(2));
}

#[test]
fn a_setext_heading_underline_is_not_a_conflict_marker() {
    // `=======` on its own line is a Markdown setext heading underline. These
    // files are prose, so treating it as a marker would refuse to write into
    // perfectly good logs.
    let contents = "Development log\n=======\n\nSome prose.\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), None);
}

#[test]
fn a_clean_month_file_has_no_conflict_marker() {
    let contents = "# Development log - 2026-04\n\n## 2026-04-17 - Entry\n\n### Result\n\n- a\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), None);
}

#[test]
fn the_diff3_ancestor_marker_is_a_conflict() {
    // `merge.conflictStyle = diff3` adds this third marker; a file carrying it
    // is as unresolved as one carrying the other two.
    let contents = "# Development log - 2026-04\n||||||| base\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), Some(2));
}

#[test]
fn markers_quoted_inside_a_fenced_block_are_not_a_conflict() {
    let contents =
        "# Development log - 2026-04\n\n```text\n<<<<<<< HEAD\n=======\n>>>>>>> feature\n```\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), None);
}

#[test]
fn a_real_conflict_after_a_fenced_block_is_still_found() {
    let contents = "# Development log - 2026-04\n\n```text\n<<<<<<< HEAD\n```\n\n>>>>>>> feature\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), Some(7));
}

#[test]
fn an_unclosed_fence_does_not_swallow_the_rest_of_the_file() {
    // A fence that never closes is malformed prose. Detection stops inside it,
    // which is the conservative direction: `check` still reports the file's
    // other structural problems rather than refusing every write to it.
    let contents = "# Development log - 2026-04\n\n~~~\n<<<<<<< HEAD\n";
    assert_eq!(nils_devlog::model::first_conflict_marker(contents), None);
}

#[test]
fn every_required_section_is_one_the_renderer_knows() {
    // `check_month` used to take the first four of `SECTIONS`, which made the
    // required set a prefix of the known set by construction. The two are now
    // independent literals, so nothing but this test stops them drifting: a
    // label only in `REQUIRED_SECTIONS` would have `check` report
    // `missing-section` on every entry in every log, for a heading `render`
    // never writes and `check` would then call `unknown-section` if an author
    // added it by hand.
    for required in nils_devlog::entry::REQUIRED_SECTIONS {
        assert!(
            nils_devlog::entry::SECTIONS.contains(&required),
            "required section {required:?} is not in SECTIONS"
        );
    }
}

#[test]
fn a_bare_url_bullet_is_rendered_as_an_autolink() {
    // MD034 is enabled in this workspace's lint baseline, so a bare URL would
    // block the commit of the entry the CLI just wrote. This assertion is the
    // contract, and it is the same shape as the MD036 one above.
    let entry = Entry {
        title: "Did a thing".to_string(),
        result: vec!["Shipped it".to_string()],
        why: vec!["It was needed".to_string()],
        evidence: vec!["Ran the gate".to_string()],
        links: vec!["https://github.com/sympoies/nils-cli/pull/1729".to_string()],
        ..Entry::default()
    };
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let rendered = entry.render(date);

    assert!(
        rendered.contains("- <https://github.com/sympoies/nils-cli/pull/1729>\n"),
        "rendered={rendered}"
    );
}

#[test]
fn a_url_is_wrapped_in_every_section_not_only_links() {
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["see https://example.com/r".to_string()],
        why: vec!["see https://example.com/w".to_string()],
        evidence: vec!["see https://example.com/e".to_string()],
        links: vec!["https://example.com/l".to_string()],
        follow_ups: vec!["see https://example.com/f".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    for suffix in ["r", "w", "e", "l", "f"] {
        assert!(
            rendered.contains(&format!("<https://example.com/{suffix}>")),
            "section {suffix} was not wrapped: {rendered}"
        );
    }
}

#[test]
fn an_already_linked_url_is_rendered_unchanged() {
    // Measured across the organization's logs, these two forms carry the
    // overwhelming majority of the URLs an author writes: 1220 inline links
    // and 454 autolinks against 74 bare URLs. Double-wrapping either would
    // break far more entries than the bare form ever blocked.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["e".to_string()],
        links: vec![
            "<https://example.com/a>".to_string(),
            "[PR 1729](https://example.com/b)".to_string(),
        ],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("- <https://example.com/a>\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains("- [PR 1729](https://example.com/b)\n"),
        "{rendered}"
    );
    assert!(!rendered.contains("<<"), "double-wrapped: {rendered}");
    assert!(!rendered.contains("(<https"), "double-wrapped: {rendered}");
}

#[test]
fn a_url_inside_a_code_span_is_left_alone() {
    // A URL in a code span is already exempt from MD034, and wrapping it would
    // change the command the entry is quoting. 39 of the URLs in the existing
    // logs sit inside one.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["ran `curl https://example.com/c` twice".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("`curl https://example.com/c`"),
        "{rendered}"
    );
    assert!(!rendered.contains("<https://example.com/c>"), "{rendered}");
}

#[test]
fn trailing_sentence_punctuation_stays_outside_the_autolink() {
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["landed in https://example.com/p.".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["e".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(rendered.contains("<https://example.com/p>."), "{rendered}");
}

#[test]
fn a_multi_backtick_code_span_is_left_alone() {
    // A span delimited by two backticks has an even backtick count, so a
    // parity-based tracker never enters it and rewrites the command the entry
    // is quoting. The delimiter is a run, not a count.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["ran ``curl https://example.com/c | sh`` twice".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("``curl https://example.com/c | sh``"),
        "{rendered}"
    );
    assert!(!rendered.contains("<https://example.com/c>"), "{rendered}");
}

#[test]
fn a_url_after_a_closed_code_span_is_still_wrapped() {
    // Quoting a command and then citing where it ran is the common shape of an
    // evidence bullet, and it is the one a tracker that never closes its span
    // would leave bare.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["ran `curl -sS` then see https://example.com/x".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("`curl -sS` then see <https://example.com/x>"),
        "{rendered}"
    );
}

#[test]
fn an_unmatched_backtick_does_not_swallow_the_rest_of_the_bullet() {
    // An unmatched run is literal text and opens nothing, per CommonMark.
    // Treating it as an opener would silently stop wrapping for the remainder,
    // which is the MD034 failure this rendering exists to prevent.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["the `-v flag, and then https://example.com/y".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(rendered.contains("<https://example.com/y>"), "{rendered}");
}

#[test]
fn a_parenthesized_url_in_prose_is_still_wrapped() {
    // `rumdl` reports a bare URL inside parentheses, and `rumdl fmt` wraps the
    // URL while leaving the parentheses outside. A word-level match that
    // required the scheme at position zero missed this entirely.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["see (https://example.com/z) for the run".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["e".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("see (<https://example.com/z>) for the run"),
        "{rendered}"
    );
}

#[test]
fn a_parenthesis_the_url_opened_stays_inside_the_autolink() {
    // The counterpart to the case above, and the reason a closing parenthesis
    // is not trimmed unconditionally: here it belongs to the URL.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["https://en.wikipedia.org/wiki/Fixture_(disambiguation)".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["e".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("- <https://en.wikipedia.org/wiki/Fixture_(disambiguation)>\n"),
        "{rendered}"
    );
}

#[test]
fn every_trailing_punctuation_character_stays_outside_the_autolink() {
    // Each of these ends a sentence rather than a URL, and `rumdl fmt` ends the
    // URL before every one of them. Pinning the whole set stops a later edit
    // from quietly dropping one.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    for mark in ['.', ',', ';', ':', '!', '?', ']'] {
        let entry = Entry {
            title: "T".to_string(),
            result: vec![format!("landed in https://example.com/p{mark} Next.")],
            why: vec!["w".to_string()],
            evidence: vec!["e".to_string()],
            ..Entry::default()
        };
        let rendered = entry.render(date);
        assert!(
            rendered.contains(&format!("<https://example.com/p>{mark}")),
            "mark={mark} rendered={rendered}"
        );
    }
}

#[test]
fn the_plaintext_scheme_is_wrapped_too() {
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["served on http://localhost:8080/health".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["e".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("<http://localhost:8080/health>"),
        "{rendered}"
    );
}

#[test]
fn a_backtick_inside_a_longer_span_does_not_close_it() {
    // Embedding a backtick is the whole reason to open a span with two of
    // them, and CommonMark closes a span only on a run of the same length. A
    // closer that accepted any run would end the span early and rewrite the
    // URL that follows inside it.
    let date: EntryDate = "2026-04-17".parse().expect("valid date");
    let entry = Entry {
        title: "T".to_string(),
        result: vec!["r".to_string()],
        why: vec!["w".to_string()],
        evidence: vec!["ran ``echo ` then curl https://example.com/d`` once".to_string()],
        ..Entry::default()
    };
    let rendered = entry.render(date);

    assert!(
        rendered.contains("``echo ` then curl https://example.com/d``"),
        "{rendered}"
    );
    assert!(!rendered.contains("<https://example.com/d>"), "{rendered}");
}
