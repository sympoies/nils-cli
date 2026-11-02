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
    /// An input region was found, but its row does not prove the exact input.
    RowUnrecognized,
}

impl Unobserved {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CaptureFailed => "capture-failed",
            Self::PaneUnparsable => "pane-unparsable",
            Self::ComposerNotLocated => "composer-not-located",
            Self::RowUnrecognized => "row-unrecognized",
        }
    }
}
