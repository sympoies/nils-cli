//! Isolated pending entries, deterministic folding, and baseline immutability.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::check::Problem;
use crate::document::{Block, Document, ID_PREFIX, blocks, invalid, valid_slug};
use crate::entry::Entry;
use crate::model::{Devlog, DevlogError, EntryDate, Month};

#[derive(Debug)]
pub(crate) struct Fragment {
    pub path: PathBuf,
    pub entry: Block,
}

pub fn enabled() -> Result<bool, DevlogError> {
    match std::env::var("DEVLOG_LAYOUT") {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Ok(value) if value.is_empty() || value == "months" => Ok(false),
        Ok(value) if value == "fragments" => Ok(true),
        _ => Err(DevlogError::InvalidLayout),
    }
}

pub(crate) fn paths(devlog: &Devlog) -> Result<Vec<PathBuf>, DevlogError> {
    let dir = devlog.dir().join("pending");
    let metadata = match std::fs::symlink_metadata(&dir) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(DevlogError::Io { path: dir, source }),
    };
    if !metadata.is_dir() {
        return Err(invalid(&dir, "pending must be a real directory"));
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|source| DevlogError::Io {
        path: dir.clone(),
        source,
    })? {
        paths.push(
            entry
                .map_err(|source| DevlogError::Io {
                    path: dir.clone(),
                    source,
                })?
                .path(),
        );
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn load(path: &Path) -> Result<Fragment, DevlogError> {
    if !std::fs::symlink_metadata(path)
        .map_err(|source| DevlogError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .is_file()
    {
        return Err(invalid(path, "fragment must be a regular file"));
    }
    let id = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".md"))
        .ok_or_else(|| invalid(path, "expected YYYY-MM-DD-slug.md"))?;
    let date = id
        .get(..10)
        .and_then(|date| date.parse::<EntryDate>().ok())
        .ok_or_else(|| invalid(path, "invalid fragment date"))?;
    let slug = id
        .get(11..)
        .filter(|slug| valid_slug(slug))
        .filter(|_| id.as_bytes().get(10) == Some(&b'-'))
        .ok_or_else(|| invalid(path, "invalid fragment slug"))?;
    let text = std::fs::read_to_string(path).map_err(|source| DevlogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let entries = blocks(&text, path)?;
    if entries.len() != 1 || !text.starts_with("## ") {
        return Err(invalid(path, "fragment must contain exactly one entry"));
    }
    let entry = entries.into_iter().next().unwrap();
    if entry.date != date || entry.identity != id || entry.slug != slug {
        return Err(invalid(
            path,
            "fragment identity must match its filename and heading date",
        ));
    }
    let mut count = 0;
    let synthetic = format!("{}\n\n{text}", date.month().heading());
    if !crate::check::check_contents(
        date.month(),
        &synthetic,
        path.display().to_string(),
        &mut count,
    )
    .is_empty()
    {
        return Err(invalid(
            path,
            "fragment entry does not satisfy the entry template",
        ));
    }
    Ok(Fragment {
        path: path.to_path_buf(),
        entry,
    })
}

pub fn write(
    devlog: &Devlog,
    entry: &Entry,
    date: EntryDate,
    slug: Option<&str>,
) -> Result<PathBuf, DevlogError> {
    let slug = match slug {
        Some(slug) if valid_slug(slug) => slug.to_string(),
        Some(_) => {
            return Err(invalid(
                devlog.dir(),
                "slug must use lowercase ASCII letters, digits and hyphens (1..180 bytes)",
            ));
        }
        None => {
            let title: String = entry
                .title
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            let title = title.trim_matches('-');
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid(devlog.dir(), "system clock is before Unix epoch"))?
                .as_nanos();
            format!(
                "{}-{nanos:x}-{:x}",
                &title[..title.len().min(100)],
                std::process::id()
            )
        }
    };
    let id = format!("{date}-{slug}");
    let rendered = entry.render(date);
    let (heading, body) = rendered.split_once('\n').unwrap();
    let text = format!("{heading}\n{ID_PREFIX}{id} -->\n{body}");
    let dir = devlog.dir().join("pending");
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
        return Err(invalid(&dir, "pending must be a real directory"));
    }
    std::fs::create_dir_all(&dir).map_err(|source| DevlogError::Io {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join(format!("{id}.md"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| DevlogError::Io {
            path: path.clone(),
            source,
        })?;
    file.write_all(text.as_bytes())
        .map_err(|source| DevlogError::Io {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}

#[derive(Debug, Serialize)]
pub struct FoldReport {
    pub folded: usize,
    pub months: Vec<String>,
    pub index_updated: bool,
}

/// Fold only dates strictly before the cutoff. The CLI uses today's local date.
pub fn fold(devlog: &Devlog, today: EntryDate) -> Result<FoldReport, DevlogError> {
    crate::index::assert_resolved(devlog)?;
    // Read the index before any mutation so a missing/unreadable index cannot
    // leave a partially folded set.
    if !std::fs::symlink_metadata(devlog.index_path())
        .map_err(|source| DevlogError::Io {
            path: devlog.index_path(),
            source,
        })?
        .is_file()
    {
        return Err(invalid(
            &devlog.index_path(),
            "fold requires a regular index file",
        ));
    }
    std::fs::read_to_string(devlog.index_path()).map_err(|source| DevlogError::Io {
        path: devlog.index_path(),
        source,
    })?;
    let fragments: Vec<_> = paths(devlog)?
        .iter()
        .map(|p| load(p))
        .collect::<Result<_, _>>()?;
    let selected: Vec<_> = fragments.iter().filter(|f| f.entry.date < today).collect();
    if selected.is_empty() {
        return Ok(FoldReport {
            folded: 0,
            months: Vec::new(),
            index_updated: false,
        });
    }
    let mut documents = BTreeMap::<Month, Document>::new();
    for fragment in &selected {
        let month = fragment.entry.date.month();
        let path = devlog.month_path(month);
        if let std::collections::btree_map::Entry::Vacant(slot) = documents.entry(month) {
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
                return Err(invalid(&path, "fold requires regular month files"));
            }
            let doc = match std::fs::read_to_string(&path) {
                Ok(contents) => Document::parse(&contents, &path)?,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Document::empty(month),
                Err(source) => return Err(DevlogError::Io { path, source }),
            };
            if doc.month != month {
                return Err(invalid(&path, "month heading disagrees with filename"));
            }
            slot.insert(doc);
        }
        let doc = documents.get_mut(&month).unwrap();
        if let Some(existing) = doc.entries.get(&fragment.entry.identity) {
            if existing != &fragment.entry {
                return Err(invalid(
                    &path,
                    "fold would overwrite an existing entry identity",
                ));
            }
        } else {
            doc.add(fragment.entry.clone(), &path)?;
        }
    }
    for (month, doc) in &documents {
        let path = devlog.month_path(*month);
        std::fs::write(&path, doc.render()).map_err(|source| DevlogError::Io { path, source })?;
    }
    for fragment in &selected {
        std::fs::remove_file(&fragment.path).map_err(|source| DevlogError::Io {
            path: fragment.path.clone(),
            source,
        })?;
    }
    let update = crate::index::sync(devlog)?;
    Ok(FoldReport {
        folded: selected.len(),
        months: documents.keys().map(ToString::to_string).collect(),
        index_updated: update.changed,
    })
}

fn git(devlog: &Devlog, args: &[&str]) -> Result<std::process::Output, DevlogError> {
    Command::new("git")
        .arg("-C")
        .arg(devlog.repo_root())
        .args(args)
        .output()
        .map_err(|source| DevlogError::Io {
            path: devlog.repo_root().to_path_buf(),
            source,
        })
}

/// Check files already present on the default branch against the working tree.
pub(crate) fn immutability(
    devlog: &Devlog,
    base: Option<&str>,
) -> Result<Vec<Problem>, DevlogError> {
    let dir = devlog.dir().strip_prefix(devlog.repo_root()).ok();
    let Some(dir) = dir else {
        return Ok(Vec::new());
    }; // External logs have no baseline in this repository.
    let pending = dir.join("pending").to_string_lossy().replace('\\', "/");
    let base = if let Some(base) = base {
        base.to_string()
    } else {
        let head = git(
            devlog,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        )?;
        if head.status.success() {
            String::from_utf8_lossy(&head.stdout).trim().to_string()
        } else if git(devlog, &["rev-parse", "--verify", "refs/heads/main"])?
            .status
            .success()
        {
            "refs/heads/main".to_string()
        } else if git(devlog, &["rev-parse", "--verify", "refs/heads/master"])?
            .status
            .success()
        {
            "refs/heads/master".to_string()
        } else if !git(devlog, &["rev-parse", "--verify", "HEAD"])?
            .status
            .success()
        {
            return Ok(Vec::new());
        } else {
            return Err(DevlogError::BaselineUnavailable);
        }
    };
    let list = git(
        devlog,
        &["ls-tree", "-rz", "--name-only", &base, "--", &pending],
    )?;
    if !list.status.success() {
        return Err(DevlogError::BaselineUnavailable);
    }
    let mut problems = Vec::new();
    for name in list.stdout.split(|b| *b == 0).filter(|n| !n.is_empty()) {
        let name = std::str::from_utf8(name)
            .map_err(|_| invalid(devlog.dir(), "non-UTF-8 baseline fragment path"))?;
        let old = git(devlog, &["show", &format!("{base}:{name}")])?;
        if !old.status.success() {
            return Err(DevlogError::BaselineUnavailable);
        }
        let path = devlog.repo_root().join(name);
        let unchanged = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file())
            && std::fs::read(&path).is_ok_and(|data| data == old.stdout);
        if unchanged {
            continue;
        }
        let deleted = !path.exists();
        let mut folded = false;
        if deleted
            && let Ok(text) = std::str::from_utf8(&old.stdout)
            && let Ok(entries) = blocks(text, &path)
            && entries.len() == 1
        {
            let entry = &entries[0];
            let month_path = devlog.month_path(entry.date.month());
            if let Ok(contents) = std::fs::read_to_string(&month_path)
                && let Ok(doc) = Document::parse(&contents, &month_path)
            {
                folded = doc.entries.get(&entry.identity) == Some(entry);
            }
        }
        if !folded {
            problems.push(Problem { kind: if deleted { "fragment-deleted" } else { "fragment-modified" }, path: name.to_string(), detail: "default-branch fragments are immutable; fold unchanged content before correcting the month entry".to_string() });
        }
    }
    Ok(problems)
}

pub(crate) fn pending_index(devlog: &Devlog) -> Result<String, DevlogError> {
    let fragments: Vec<_> = paths(devlog)?
        .iter()
        .map(|p| load(p))
        .collect::<Result<_, _>>()?;
    let mut fragments = fragments;
    fragments.sort_by(|a, b| {
        b.entry
            .date
            .cmp(&a.entry.date)
            .then(a.entry.slug.cmp(&b.entry.slug))
    });
    Ok(fragments
        .iter()
        .map(|fragment| {
            let name = fragment.path.file_name().unwrap().to_string_lossy();
            format!("- [{}](pending/{name})", fragment.entry.identity)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
