//! Three-way union of entry identities for a local Git merge driver.
use std::collections::BTreeSet;
use std::path::Path;

use crate::document::{Document, invalid};
use crate::model::DevlogError;

fn read(path: &Path) -> Result<String, DevlogError> {
    std::fs::read_to_string(path).map_err(|source| DevlogError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Overwrite `ours` only after all entries and concurrent corrections agree.
/// A same-identity edit on one side is kept; incompatible edits fail closed.
pub fn merge(base: &Path, ours: &Path, theirs: &Path) -> Result<(), DevlogError> {
    let ours_doc = Document::parse(&read(ours)?, ours)?;
    let theirs_doc = Document::parse(&read(theirs)?, theirs)?;
    let base_text = read(base)?;
    let base_doc = if base_text.trim().is_empty() {
        Document::empty(ours_doc.month)
    } else {
        Document::parse(&base_text, base)?
    };
    if ours_doc.month != theirs_doc.month || ours_doc.month != base_doc.month {
        return Err(invalid(ours, "merge inputs belong to different months"));
    }
    let preamble =
        if ours_doc.preamble == theirs_doc.preamble || theirs_doc.preamble == base_doc.preamble {
            ours_doc.preamble.clone()
        } else if ours_doc.preamble == base_doc.preamble {
            theirs_doc.preamble.clone()
        } else {
            return Err(invalid(ours, "both sides changed the month preamble"));
        };
    let mut merged = Document::empty(ours_doc.month);
    merged.preamble = preamble;
    let identities: BTreeSet<_> = ours_doc
        .entries
        .keys()
        .chain(theirs_doc.entries.keys())
        .collect();
    for id in identities {
        let a = ours_doc.entries.get(id);
        let b = theirs_doc.entries.get(id);
        let old = base_doc.entries.get(id);
        let entry = match (a, b) {
            (Some(a), Some(b)) if a == b || Some(b) == old => a,
            (Some(a), Some(b)) if Some(a) == old => b,
            (Some(_), Some(_)) => {
                return Err(invalid(
                    ours,
                    format!("both sides changed entry identity '{id}'"),
                ));
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => unreachable!("identity came from one of the sides"),
        };
        merged.add(entry.clone(), ours)?;
    }
    std::fs::write(ours, merged.render()).map_err(|source| DevlogError::Io {
        path: ours.to_path_buf(),
        source,
    })
}
