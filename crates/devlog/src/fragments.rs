//! Isolated pending entries, deterministic folding, and baseline immutability.
use std::collections::{BTreeMap, BTreeSet};
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
    let index_contents =
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
    let mut months = devlog.months()?.months;
    months.extend(documents.keys().copied());
    months.sort();
    months.dedup();
    let pending = render_pending(fragments.iter().filter(|f| f.entry.date >= today));
    let updated_index = crate::index::render_index(&index_contents, &months, &pending);
    let index_updated = updated_index != index_contents;
    for (month, doc) in &documents {
        let path = devlog.month_path(*month);
        std::fs::write(&path, doc.render()).map_err(|source| DevlogError::Io { path, source })?;
    }
    // Keep every source until both destinations exist. If a write fails, retry
    // recognizes an already-copied identity and finishes the same fold.
    if index_updated {
        std::fs::write(devlog.index_path(), &updated_index).map_err(|source| DevlogError::Io {
            path: devlog.index_path(),
            source,
        })?;
    }
    for fragment in &selected {
        std::fs::remove_file(&fragment.path).map_err(|source| DevlogError::Io {
            path: fragment.path.clone(),
            source,
        })?;
    }
    Ok(FoldReport {
        folded: selected.len(),
        months: documents.keys().map(ToString::to_string).collect(),
        index_updated,
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
    immutability_with_fold(devlog, base, None)
}

fn immutability_with_fold(
    devlog: &Devlog,
    base: Option<&str>,
    pr_changed: Option<&BTreeSet<String>>,
) -> Result<Vec<Problem>, DevlogError> {
    // Compare physical paths without changing the established --dir display
    // behavior. Relative directories are resolved against the caller's cwd.
    let root = std::fs::canonicalize(devlog.repo_root()).map_err(|source| DevlogError::Io {
        path: devlog.repo_root().to_path_buf(),
        source,
    })?;
    let directory = std::fs::canonicalize(devlog.dir()).map_err(|source| DevlogError::Io {
        path: devlog.dir().to_path_buf(),
        source,
    })?;
    let Some(dir) = directory.strip_prefix(&root).ok() else {
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
    check_immutable_paths(devlog, &base, &list.stdout, pr_changed)
}

fn check_immutable_paths(
    devlog: &Devlog,
    base: &str,
    names: &[u8],
    pr_changed: Option<&BTreeSet<String>>,
) -> Result<Vec<Problem>, DevlogError> {
    let mut problems = Vec::new();
    for name in names.split(|b| *b == 0).filter(|n| !n.is_empty()) {
        let name = std::str::from_utf8(name)
            .map_err(|_| invalid(devlog.dir(), "non-UTF-8 baseline fragment path"))?;
        let old = git(devlog, &["show", &format!("{base}:{name}")])?;
        if !old.status.success() {
            return Err(DevlogError::BaselineUnavailable);
        }
        let path = devlog.repo_root().join(name);
        let unchanged = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file())
            && std::fs::read(&path).is_ok_and(|data| data == old.stdout);
        if unchanged && !pr_changed.is_some_and(|changed| changed.contains(name)) {
            continue;
        }
        let deleted = !path.exists();
        let mut folded = false;
        if pr_changed.is_none()
            && deleted
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
            problems.push(Problem {
                kind: if deleted { "fragment-deleted" } else { "fragment-modified" },
                path: name.to_string(),
                detail: if pr_changed.is_some() {
                    "merged fragments cannot change in a PR; only the trusted fold owner may delete them"
                } else {
                    "default-branch fragments are immutable; fold unchanged content before correcting the month entry"
                }.to_string(),
            });
        }
    }
    Ok(problems)
}

/// Resolve both refs as commits before passing them to merge-base. Missing or
/// shallow unrelated history must never turn PR enforcement into a clean check.
pub(crate) fn merge_base(devlog: &Devlog, base: &str) -> Result<String, DevlogError> {
    let resolved = git(
        devlog,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )?;
    if !resolved.status.success() {
        return Err(DevlogError::BaselineUnavailable);
    }
    let commit = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
    let merge = git(devlog, &["merge-base", "HEAD", &commit])?;
    if !merge.status.success() {
        return Err(DevlogError::BaselineUnavailable);
    }
    Ok(String::from_utf8_lossy(&merge.stdout).trim().to_string())
}

/// Check each Git layer separately: comparing only the final working tree to
/// the base would hide an index or HEAD change restored in a later layer.
pub(crate) fn pr_changes(devlog: &Devlog, base: &str) -> Result<Vec<Problem>, DevlogError> {
    let root = std::fs::canonicalize(devlog.repo_root()).map_err(|source| DevlogError::Io {
        path: devlog.repo_root().to_path_buf(),
        source,
    })?;
    let directory = std::fs::canonicalize(devlog.dir()).map_err(|source| DevlogError::Io {
        path: devlog.dir().to_path_buf(),
        source,
    })?;
    let resolved_dir = directory.strip_prefix(&root).map_err(|_| {
        invalid(
            devlog.dir(),
            "--fragments-only requires a log inside this repository",
        )
    })?;
    // Canonical paths prove containment, while lexical paths retain ownership
    // when a branch replaces a tracked directory with a directory symlink.
    let absolute = if devlog.dir().is_absolute() {
        devlog.dir().to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| DevlogError::Io {
                path: devlog.dir().to_path_buf(),
                source,
            })?
            .join(devlog.dir())
    };
    let mut lexical = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            _ => lexical.push(component.as_os_str()),
        }
    }
    let logical_dir = lexical.strip_prefix(&root).map_err(|_| {
        invalid(
            devlog.dir(),
            "--fragments-only requires a repository-relative log path",
        )
    })?;
    let mut dirs = BTreeSet::from([resolved_dir.to_path_buf(), logical_dir.to_path_buf()]);
    if crate::model::DEVLOG_DIRS
        .iter()
        .any(|candidate| dirs.contains(Path::new(candidate)))
    {
        // Keep both conventions visible even if the feature branch deletes or
        // shadows the directory that discovery would select at the merge base.
        dirs.extend(crate::model::DEVLOG_DIRS.iter().map(PathBuf::from));
    }
    let pathspecs: Vec<_> = dirs
        .iter()
        .map(|dir| format!(":(literal){}", dir.to_string_lossy().replace('\\', "/")))
        .collect();
    let mut changed = BTreeSet::new();
    for mut args in [
        vec![
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            base,
            "HEAD",
            "--",
        ],
        vec![
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "--",
        ],
        vec!["diff", "--name-only", "--no-renames", "-z", "--"],
        vec!["ls-files", "--others", "-z", "--"],
    ] {
        args.extend(pathspecs.iter().map(String::as_str));
        let output = git(devlog, &args)?;
        if !output.status.success() {
            return Err(DevlogError::BaselineUnavailable);
        }
        for name in output.stdout.split(|b| *b == 0).filter(|n| !n.is_empty()) {
            changed.insert(
                std::str::from_utf8(name)
                    .map_err(|_| invalid(devlog.dir(), "non-UTF-8 changed log path"))?
                    .to_string(),
            );
        }
    }
    let pending: Vec<_> = dirs
        .iter()
        .map(|dir| dir.join("pending").to_string_lossy().replace('\\', "/"))
        .collect();
    let mut args = vec!["ls-tree", "-rz", "--name-only", base, "--"];
    args.extend(pending.iter().map(String::as_str));
    let baseline = git(devlog, &args)?;
    if !baseline.status.success() {
        return Err(DevlogError::BaselineUnavailable);
    }
    let mut problems = check_immutable_paths(devlog, base, &baseline.stdout, Some(&changed))?;
    for name in changed {
        let path = Path::new(&name);
        if path.parent().is_some_and(|parent| dirs.contains(parent))
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_suffix(".md"))
                .is_some_and(|s| s.parse::<Month>().is_ok())
        {
            problems.push(Problem { kind: "month-file-changed", path: name, detail: "month files cannot change in a fragment-only PR; only the trusted fold owner may update them".to_string() });
        }
    }
    Ok(problems)
}

pub(crate) fn pending_index(devlog: &Devlog) -> Result<String, DevlogError> {
    let fragments: Vec<_> = paths(devlog)?
        .iter()
        .map(|p| load(p))
        .collect::<Result<_, _>>()?;
    Ok(render_pending(fragments.iter()))
}

fn render_pending<'a>(fragments: impl Iterator<Item = &'a Fragment>) -> String {
    let mut fragments: Vec<_> = fragments.collect();
    fragments.sort_by(|a, b| {
        b.entry
            .date
            .cmp(&a.entry.date)
            .then(a.entry.slug.cmp(&b.entry.slug))
    });
    fragments
        .iter()
        .map(|fragment| {
            let name = fragment.path.file_name().unwrap().to_string_lossy();
            format!("- [{}](pending/{name})", fragment.entry.identity)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
