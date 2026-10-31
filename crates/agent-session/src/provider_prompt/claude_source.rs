//! Resolve a known primary Claude conversation before scanning unrelated metadata.

use std::collections::BTreeSet;
use std::path::Component;

use nils_provider_resume::{ClaudeResumeScanBudget, collect_claude_provider_resume_matches};

use super::{
    Path, PathBuf, file_identity, fs, open_regular_file, opened_file_still_current,
    read_claude_session_cwd,
};

/// Outcome of looking up a known primary Claude conversation. Only `NotFound`
/// is a completed lookup that proved no candidate exists; everything ambiguous,
/// unsafe or incomplete is `Unresolved`.
pub(super) enum Lookup {
    Found(PathBuf),
    NotFound,
    Unresolved,
}

pub(super) fn resolve(root: &Path, session_id: &str) -> Option<PathBuf> {
    match lookup(root, session_id) {
        Lookup::Found(path) => Some(path),
        Lookup::NotFound | Lookup::Unresolved => None,
    }
}

pub(super) fn lookup(root: &Path, session_id: &str) -> Lookup {
    // The root comes from the session's configuration binding. Resolve its
    // account link without following project or transcript links during discovery.
    let canonical_root = match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::canonicalize(root) {
            Ok(root) => Some(root),
            Err(_) => return Lookup::Unresolved,
        },
        Ok(_) => None,
        Err(_) => return Lookup::Unresolved,
    };
    let root = canonical_root.as_deref().unwrap_or(root);
    match known_source(root, session_id) {
        Ok(Some(path)) => Lookup::Found(path),
        // Retain discovery for older/noncanonical transcript filenames, but
        // never turn an incomplete or unsafe exact lookup into a fallback win.
        Ok(None) => {
            let mut matches = BTreeSet::new();
            let mut budget = ClaudeResumeScanBudget::from_env();
            collect_claude_provider_resume_matches(root, session_id, &mut matches, &mut budget);
            match (budget.truncated, matches.len()) {
                (true, _) => Lookup::Unresolved,
                (false, 0) => Lookup::NotFound,
                (false, 1) => matches
                    .into_iter()
                    .next()
                    .map_or(Lookup::Unresolved, |found| Lookup::Found(found.path)),
                (false, _) => Lookup::Unresolved,
            }
        }
        Err(()) => Lookup::Unresolved,
    }
}

fn known_source(root: &Path, session_id: &str) -> Result<Option<PathBuf>, ()> {
    let mut components = Path::new(session_id).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || session_id.contains(['/', '\\'])
        || session_id.chars().any(char::is_control)
    {
        return Err(());
    }
    if !fs::symlink_metadata(root)
        .map_err(|_| ())?
        .file_type()
        .is_dir()
    {
        return Err(());
    }
    let filename = format!("{session_id}.jsonl");
    let mut budget = ClaudeResumeScanBudget::from_env();
    let mut found = None;
    for project in fs::read_dir(root).map_err(|_| ())? {
        if !budget.visit_entry() {
            return Err(());
        }
        let project = project.map_err(|_| ())?;
        if !project.file_type().map_err(|_| ())?.is_dir() {
            continue;
        }
        // Probe one known filename per project; do not enumerate or parse the
        // project's unrelated transcripts just to locate this conversation.
        let path = project.path().join(&filename);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(()),
            Ok(metadata) if !metadata.file_type().is_file() => return Err(()),
            Ok(_) => {}
        }
        if !budget.visit_entry() {
            return Err(());
        }
        let (_, metadata) = open_regular_file(&path).map_err(|_| ())?;
        if read_claude_session_cwd(&path, session_id).is_none()
            || !opened_file_still_current(&path, file_identity(&metadata))
            || std::time::Instant::now() >= budget.deadline
            || found.is_some()
        {
            return Err(());
        }
        found = Some(path);
    }
    // A truncated project walk cannot prove that the source is unique.
    if std::time::Instant::now() >= budget.deadline {
        return Err(());
    }
    Ok(found)
}
