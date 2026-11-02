//! Summary admission is separate from rendering for the bounded internal mailbox producer.
use crate::CliError;

pub(super) fn validate(summary: &str) -> Result<(), CliError> {
    if !(1..=70).contains(&summary.chars().count())
        || !summary.chars().any(|c| !c.is_whitespace())
        || summary.chars().any(|c| c <= '\u{1f}' || matches!(c, '\u{7f}' | '\u{85}' | '\u{2028}' | '\u{2029}'))
    {
        return Err(CliError::with_exit_code(
            "notice-summary-invalid",
            "notice summary must contain 1..70 Unicode scalar values, include non-whitespace text, and contain no controls or line separators",
            None,
            2,
        ));
    }
    Ok(())
}

/// Internal mailbox counts are quota-bounded and exempt from CLI scalar admission.
pub(crate) fn render_summary(kind: &str, summary: &str, body: &str) -> String {
    let mut rendered = format!("{}: {summary}", super::labels::label_for(kind));
    if !body.is_empty() {
        rendered.push('\n');
        rendered.push_str(body);
    }
    rendered
}
