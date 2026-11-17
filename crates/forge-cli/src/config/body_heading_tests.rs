use super::ForgeConfig;

#[test]
fn invalid_heading_values_warn_and_are_rejected() {
    let mut failures = Vec::new();
    for key in ["summary_heading", "test_plan_heading"] {
        for value in [
            "",
            "   ",
            "# Title",
            "### Title",
            "## ",
            "## ###",
            "## Title ",
            "## Title\nOther",
            "## Title\rOther",
        ] {
            let cfg = ForgeConfig::parse_content(&format!("[body]\n{key} = {value:?}\n")).unwrap();
            let warned = cfg
                .warnings
                .iter()
                .any(|w| w.contains(&format!("invalid-config-value:body.{key}")));
            let rejected = crate::validations::BodyHeadings::validate_config(&cfg).is_err();
            if !warned || !rejected {
                failures.push(format!(
                    "{key}={value:?}: warned={warned} rejected={rejected}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn equal_resolved_headings_are_rejected_after_layering_and_defaults() {
    let mut failures = Vec::new();
    for (base, top) in [
        (
            "",
            "[body]\nsummary_heading = '## Same'\ntest_plan_heading = '## Same'",
        ),
        (
            "[body]\nsummary_heading = '## Same'",
            "[body]\ntest_plan_heading = '## Same'",
        ),
        ("", "[body]\nsummary_heading = '## Test plan'"),
        ("", "[body]\ntest_plan_heading = '## Summary'"),
    ] {
        let cfg = ForgeConfig::parse_content(base)
            .unwrap()
            .overlaid_by(ForgeConfig::parse_content(top).unwrap());
        let warned = cfg
            .warnings
            .iter()
            .any(|w| w.contains("body.summary_heading") && w.contains("body.test_plan_heading"));
        if !warned || crate::validations::BodyHeadings::validate_config(&cfg).is_ok() {
            failures.push(format!("{base:?} {top:?}: {:?}", cfg.warnings));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn named_headings_with_closing_hashes_are_valid() {
    for heading in ["## Notes ##", "## Notes ###"] {
        let cfg = ForgeConfig::parse_content(&format!("[body]\nsummary_heading = {heading:?}\n"))
            .unwrap();
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        assert!(crate::validations::BodyHeadings::validate_config(&cfg).is_ok());
    }
}
