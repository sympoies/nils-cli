//! Lossless entry blocks shared by folding and the three-way merge driver.
use std::collections::BTreeMap;
use std::path::Path;

use crate::model::{DevlogError, EntryDate, Month, first_conflict_marker, structural_line_mask};

pub(crate) const ID_PREFIX: &str = "<!-- devlog-id: ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub date: EntryDate,
    pub slug: String,
    pub identity: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub month: Month,
    pub preamble: String,
    pub entries: BTreeMap<String, Block>,
}

pub(crate) fn invalid(path: &Path, detail: impl Into<String>) -> DevlogError {
    DevlogError::InvalidContent {
        path: path.to_path_buf(),
        detail: detail.into(),
    }
}

pub(crate) fn blocks(contents: &str, path: &Path) -> Result<Vec<Block>, DevlogError> {
    if let Some(line) = first_conflict_marker(contents) {
        return Err(DevlogError::ConflictMarkers {
            path: path.to_path_buf(),
            line,
        });
    }
    if crate::model::has_unterminated_fence(contents.lines()) {
        return Err(invalid(path, "unterminated fenced code block"));
    }
    let mask = structural_line_mask(contents.lines());
    let mut offsets = Vec::new();
    let mut offset = 0;
    for (i, line) in contents.split_inclusive('\n').enumerate() {
        if mask[i] && line.starts_with("## ") {
            offsets.push(offset);
        }
        offset += line.len();
    }
    let mut out = Vec::new();
    for (i, start) in offsets.iter().enumerate() {
        let end = offsets.get(i + 1).copied().unwrap_or(contents.len());
        let text = contents[*start..end].trim_end().to_string() + "\n";
        let heading = text
            .lines()
            .next()
            .unwrap()
            .trim_end()
            .strip_prefix("## ")
            .unwrap();
        let (date, title) = heading
            .split_once(" - ")
            .ok_or_else(|| invalid(path, "malformed entry heading"))?;
        let date = date
            .parse::<EntryDate>()
            .map_err(|_| invalid(path, "invalid entry date"))?;
        let marker = text
            .lines()
            .nth(1)
            .and_then(|line| line.strip_prefix(ID_PREFIX))
            .and_then(|value| value.strip_suffix(" -->"));
        let (identity, slug) = match marker {
            Some(id) => {
                let prefix = format!("{date}-");
                let slug = id
                    .strip_prefix(&prefix)
                    .filter(|slug| valid_slug(slug))
                    .ok_or_else(|| invalid(path, "invalid devlog identity"))?;
                (id.to_string(), slug.to_string())
            }
            None => (format!("heading:{date}:{title}"), title.to_string()),
        };
        out.push(Block {
            date,
            slug,
            identity,
            text,
        });
    }
    Ok(out)
}

pub(crate) fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 180
        && slug
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

impl Document {
    pub fn empty(month: Month) -> Self {
        Self {
            month,
            preamble: month.heading() + "\n",
            entries: BTreeMap::new(),
        }
    }
    pub fn parse(contents: &str, path: &Path) -> Result<Self, DevlogError> {
        let heading = contents.lines().next().unwrap_or_default();
        let month = heading
            .strip_prefix("# Development log - ")
            .and_then(|m| m.parse::<Month>().ok())
            .ok_or_else(|| invalid(path, "expected a month heading"))?;
        let entries = blocks(contents, path)?;
        let mask = structural_line_mask(contents.lines());
        let mut first = contents.len();
        let mut offset = 0;
        for (i, line) in contents.split_inclusive('\n').enumerate() {
            if mask[i] && line.starts_with("## ") {
                first = offset;
                break;
            }
            offset += line.len();
        }
        let mut doc = Self {
            month,
            preamble: contents[..first].trim_end().to_string() + "\n",
            entries: BTreeMap::new(),
        };
        for entry in entries {
            doc.add(entry, path)?;
        }
        Ok(doc)
    }
    pub fn add(&mut self, entry: Block, path: &Path) -> Result<(), DevlogError> {
        if entry.date.month() != self.month {
            return Err(invalid(path, "entry date does not belong to month"));
        }
        if let Some(existing) = self.entries.get(&entry.identity) {
            if existing != &entry {
                return Err(invalid(
                    path,
                    "different contents for the same entry identity",
                ));
            }
            return Err(invalid(path, "duplicate entry identity"));
        }
        self.entries.insert(entry.identity.clone(), entry);
        Ok(())
    }
    pub fn render(&self) -> String {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| {
            b.date
                .cmp(&a.date)
                .then(a.slug.cmp(&b.slug))
                .then(a.identity.cmp(&b.identity))
        });
        let mut out = self.preamble.trim_end().to_string() + "\n";
        for entry in entries {
            out.push('\n');
            out.push_str(&entry.text);
        }
        out
    }
}
