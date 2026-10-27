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
        if self.args.notice_envelope
            && self.record.agent == "claude"
            && crate::notice::parse_notice(text).is_some_and(|notice| {
                notice
                    .body
                    .split('\n')
                    .any(|row| row.strip_suffix('\r').unwrap_or(row).ends_with(' '))
            })
        {
            return Err(self.refusal(
                "send-notice-trailing-spaces",
                "Claude terminal notices with body rows ending in ASCII spaces cannot be verified exactly; remove the trailing spaces before retrying",
                "text",
                "admission",
            ));
        }
        if !self.verify_composer {
            return Ok(text);
        }
        if self.args.notice_envelope {
            return Ok(text);
        }
        // A bare `\n`, `\r` or `\r\n` is a key press and never reaches this check.
        let trimmed = text.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() || trimmed.contains(['\n', '\r']) {
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
