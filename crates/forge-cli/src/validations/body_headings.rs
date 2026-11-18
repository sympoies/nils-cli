use super::BodyHeadings;
use crate::config::ForgeConfig;

impl BodyHeadings {
    /// Resolve both section headings through the existing configuration defaults.
    pub(crate) fn from_config(config: &ForgeConfig) -> Self {
        Self {
            summary: config.resolve_summary_heading(None),
            test_plan: config.resolve_test_plan_heading(None),
        }
    }
}

impl BodyHeadings {
    pub(crate) fn validate_config(config: &ForgeConfig) -> Result<(), crate::error::ForgeError> {
        let summary = config.resolve_summary_heading(None);
        let test_plan = config.resolve_test_plan_heading(None);
        let mut invalid = Vec::new();
        if !crate::config::valid_body_heading(&summary) {
            invalid.push("invalid-config-value:body.summary_heading:expected_non_empty_h2");
        }
        if !crate::config::valid_body_heading(&test_plan) {
            invalid.push("invalid-config-value:body.test_plan_heading:expected_non_empty_h2");
        }
        if summary == test_plan {
            invalid.push("invalid-config-value:body.summary_heading:body.test_plan_heading:headings_must_differ");
        }
        if invalid.is_empty() {
            Ok(())
        } else {
            Err(crate::error::ForgeError::validation(
                nils_common::cli_contract::schema_version_for(crate::cli::BINARY, "error", 1),
                "invalid_config_value",
                "invalid PR body heading configuration; use distinct non-empty H2 headings",
                Some(invalid.join("; ")),
            ))
        }
    }
}
