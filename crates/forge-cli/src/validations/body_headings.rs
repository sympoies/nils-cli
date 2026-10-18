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
