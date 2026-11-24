//! Account-local Claude transcript import for an admitted destination.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{CliError, claude_account};

use super::{History, Refusal, canonical_uuid, cli_refusal, sha256};

const MAX_TRANSCRIPT_ROW_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROJECTS: usize = 10_000;

pub(super) struct Destination {
    account_root: PathBuf,
    projects: PathBuf,
    transcript: PathBuf,
}

impl Destination {
    pub(super) fn for_account(account: &str, cwd: &Path, thread: &str) -> Result<Self, CliError> {
        let scoped = claude_account::history::root(account)?;
        Self::observe(&scoped.config_dir, cwd, thread)
    }

    pub(super) fn observe(account: &Path, cwd: &Path, thread: &str) -> Result<Self, CliError> {
        let account_root = fs::canonicalize(account).map_err(|_| path_mismatch())?;
        validate_directory(&account_root)?;
        let projects = projects_root(&account_root)?;
        check_no_transcript(&projects, thread)?;
        let transcript = projects
            .join(project_slug(cwd)?)
            .join(transcript_name(thread)?);
        // If the checkout directory already exists, verify it now without writing.
        let directory = transcript.parent().ok_or_else(path_mismatch)?;
        match fs::symlink_metadata(directory) {
            Ok(_) => validate_directory(directory)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(path_mismatch()),
        }
        Ok(Self {
            account_root,
            projects,
            transcript,
        })
    }

    /// Only B's broker can materialize the B account. Its credential file
    /// is never part of the move payload or materialization input.
    pub(super) fn prepare_account(&self, account: &str) -> Result<PathBuf, CliError> {
        let prepared = claude_account::materialize(account)?;
        let prepared = fs::canonicalize(prepared).map_err(|_| path_mismatch())?;
        if self.account_root != prepared || self.projects != projects_root(&prepared)? {
            return Err(path_mismatch());
        }
        Ok(prepared)
    }
}

#[cfg(test)]
pub(super) fn project_path(account: &Path, cwd: &Path, thread: &str) -> Result<PathBuf, CliError> {
    Ok(projects_root(account)?
        .join(project_slug(cwd)?)
        .join(transcript_name(thread)?))
}

fn projects_root(account: &Path) -> Result<PathBuf, CliError> {
    let projects = account.join("projects");
    match fs::symlink_metadata(&projects) {
        Ok(_) => {
            // The account broker may intentionally configure a shared
            // projects directory. Use only its B-local canonical target.
            let canonical = fs::canonicalize(projects).map_err(|_| path_mismatch())?;
            validate_directory(&canonical)?;
            Ok(canonical)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(projects),
        Err(_) => Err(path_mismatch()),
    }
}

fn project_slug(cwd: &Path) -> Result<String, CliError> {
    let cwd = cwd.to_str().ok_or_else(path_mismatch)?;
    if !cwd.starts_with('/')
        || cwd.len() > 200
        || cwd
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return Err(path_mismatch());
    }
    // Claude's local projects layout encodes the canonical checkout path.
    // The v1 move contract accepts the short ASCII encoding only.
    Ok(cwd
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect())
}

fn transcript_name(thread: &str) -> Result<String, CliError> {
    if !canonical_uuid(thread) {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    Ok(format!("{thread}.jsonl"))
}

fn validate_directory(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| path_mismatch())?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(path_mismatch());
    }
    Ok(())
}

fn check_no_transcript(projects: &Path, thread: &str) -> Result<(), CliError> {
    let name = transcript_name(thread)?;
    let entries = match fs::read_dir(projects) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(path_mismatch()),
    };
    for (index, entry) in entries.enumerate() {
        if index == MAX_PROJECTS {
            return Err(path_mismatch());
        }
        let entry = entry.map_err(|_| path_mismatch())?;
        let kind = entry.file_type().map_err(|_| path_mismatch())?;
        if kind.is_symlink() {
            return Err(path_mismatch());
        }
        if !kind.is_dir() {
            continue;
        }
        match fs::symlink_metadata(entry.path().join(&name)) {
            Ok(_) => return Err(cli_refusal(Refusal::DestinationWriterPresent)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(path_mismatch()),
        }
    }
    Ok(())
}

pub(super) fn check_source_dependencies(
    account: &Path,
    transcript: &Path,
    thread: &str,
) -> Result<(), CliError> {
    let directory = transcript.parent().ok_or_else(path_mismatch)?.join(thread);
    for dependency in [
        directory.join("subagents"),
        directory.join("tool-results"),
        directory.join("mcp-tasks"),
        account.join("uploads").join(thread),
        account.join("image-cache").join(thread),
    ] {
        match fs::symlink_metadata(dependency) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(unsupported_history()),
        }
    }
    Ok(())
}

pub(super) fn transcript_cwd_digest(file: &mut File, thread: &str) -> Result<String, CliError> {
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut checkout = None;
    loop {
        line.clear();
        let count = (&mut reader)
            .take(MAX_TRANSCRIPT_ROW_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| unsupported_history())?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_TRANSCRIPT_ROW_BYTES {
            return Err(unsupported_history());
        }
        let row: Value = serde_json::from_slice(&line).map_err(|_| unsupported_history())?;
        if !row.is_object() || row["isSidechain"] == true || external_attachment(&row) {
            return Err(unsupported_history());
        }
        let session = row
            .get("sessionId")
            .or_else(|| row.get("session_id"))
            .and_then(Value::as_str);
        if let Some(session) = session {
            if session != thread {
                return Err(cli_refusal(Refusal::SessionMismatch));
            }
            if let Some(cwd) = row["cwd"].as_str() {
                if !Path::new(cwd).is_absolute() {
                    return Err(path_mismatch());
                }
                let digest = sha256(cwd.as_bytes());
                if checkout.as_ref().is_some_and(|prior| prior != &digest) {
                    return Err(path_mismatch());
                }
                checkout = Some(digest);
            }
        }
    }
    checkout.ok_or_else(path_mismatch)
}

fn external_attachment(row: &Value) -> bool {
    if row["type"] != "attachment" {
        return false;
    }
    let attachment = &row["attachment"];
    attachment.get("filePath").is_some()
        || attachment.get("path").is_some()
        || attachment.get("taskId").is_some()
}

pub(super) fn install_transcript(
    target: &Destination,
    staged: &Path,
    expected: &History,
) -> Result<(), CliError> {
    let directory = target.transcript.parent().ok_or_else(path_mismatch)?;
    for path in [&target.projects, directory] {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {
                sync_directory(path.parent().ok_or_else(path_mismatch)?)?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(path_mismatch()),
        }
        validate_directory(path)?;
    }

    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(staged)
        .map_err(|_| history_stale())?;
    let metadata = input.metadata().map_err(|_| history_stale())?;
    if !metadata.is_file() || metadata.len() != expected.bytes {
        return Err(history_stale());
    }
    let mut reader = input.take(expected.bytes.saturating_add(1));
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|_| path_mismatch())?;
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).map_err(|_| history_stale())?;
        if count == 0 {
            break;
        }
        let chunk = &buffer[..count];
        hasher.update(chunk);
        temporary.write_all(chunk).map_err(|_| path_mismatch())?;
        bytes += count as u64;
    }
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes != expected.bytes || hash != expected.sha256 {
        return Err(history_stale());
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| path_mismatch())?;
    temporary
        .persist_noclobber(&target.transcript)
        .map_err(|failure| {
            if failure.error.kind() == io::ErrorKind::AlreadyExists {
                cli_refusal(Refusal::DestinationWriterPresent)
            } else {
                path_mismatch()
            }
        })?;
    sync_directory(directory)
}

fn sync_directory(directory: &Path) -> Result<(), CliError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| path_mismatch())
}

fn unsupported_history() -> CliError {
    cli_refusal(Refusal::HistoryUnsupported)
}

fn history_stale() -> CliError {
    cli_refusal(Refusal::HistoryStale)
}

fn path_mismatch() -> CliError {
    cli_refusal(Refusal::PathMismatch)
}
