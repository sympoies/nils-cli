//! Which observation step left a composer unreadable. The tokens are fixed
//! words that never carry pane text, paths or raw output.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Unobserved {
    /// The terminal capture command failed or timed out.
    CaptureFailed,
    /// The capture holds an escape or control sequence the reader does not accept.
    PaneUnparsable,
    /// No input region was found, as when a dialog or picker replaces the composer.
    ComposerNotLocated,
    /// The input region spans more than one row.
    RegionNotSingleRow,
    /// A dialog hint sits in the footer below the input region.
    FooterDialogHint,
    /// A queued-reply hint where the caller does not admit one.
    QueuedHintNotAdmitted,
    /// The input row shows a paste placeholder.
    PastePlaceholder,
    /// The input row is a suggestion's wording without dim styling.
    SuggestionLiteralUnstyled,
    /// The region's row does not start with the prompt glyph or cannot be located.
    RowUnrecognized,
}

impl Unobserved {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CaptureFailed => "capture-failed",
            Self::PaneUnparsable => "pane-unparsable",
            Self::ComposerNotLocated => "composer-not-located",
            Self::RegionNotSingleRow => "region-not-single-row",
            Self::FooterDialogHint => "footer-dialog-hint",
            Self::QueuedHintNotAdmitted => "queued-hint-not-admitted",
            Self::PastePlaceholder => "paste-placeholder",
            Self::SuggestionLiteralUnstyled => "suggestion-literal-unstyled",
            Self::RowUnrecognized => "row-unrecognized",
        }
    }
}
