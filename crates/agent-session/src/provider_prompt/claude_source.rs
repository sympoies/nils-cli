//! Resolve a known primary Claude conversation before scanning unrelated metadata.

use std::path::Component;

use nils_provider_resume::ClaudeResumeScanBudget;

use super::{
    AgentKind, Path, PathBuf, file_identity, fs, open_regular_file, opened_file_still_current,
    read_claude_session_cwd, resolve_provider_transcript_path_from_roots,
};

pub(super) fn resolve(root: &Path, session_id: &str) -> Option<PathBuf> {
    match known_source(root, session_id) {
        Ok(Some(path)) => Some(path),
        // Retain discovery for older/noncanonical transcript filenames, but
        // never turn an incomplete or unsafe exact lookup into a fallback win.
        Ok(None) => resolve_provider_transcript_path_from_roots(
            AgentKind::Claude,
            session_id,
            None,
            Some(root),
        ),
        Err(()) => None,
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
