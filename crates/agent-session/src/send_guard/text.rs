//! Which guarded text the single-row composer check can verify after a paste.
use super::Admission;
use crate::CliError;

impl Admission<'_> {
    /// The text a guarded text-and-key send may paste. Claude and Codex draw a
    /// pasted line break as another composer row, which the exact single-row
    /// check cannot read back, so a send that verifies its paste drops the
    /// trailing line breaks (a submitted prompt ignores them) and refuses any
    /// other line break before pasting instead of after it.
    pub(crate) fn observable_text<'t>(&self, text: &'t str) -> Result<&'t str, CliError> {
        if !self.verify_composer {
            return Ok(text);
        }
        let trimmed = text.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return Ok(text);
        }
        if trimmed.contains(['\n', '\r']) {
            return Err(self.refusal(
                "send-text-multiline",
                "multi-line text cannot be verified in the composer; send one line",
                "text",
                "admission",
            ));
        }
        Ok(trimmed)
    }
}
