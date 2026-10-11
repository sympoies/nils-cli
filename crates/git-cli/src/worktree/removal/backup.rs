//! Object-backed worktree preservation; the real index is never the snapshot index.
use super::{CliError, Fence, git, git_error_reason, lease, probe, probe_error_reason, refused};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::worktree) struct Omission {
    pub(in crate::worktree) path: String,
    pub(in crate::worktree) bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    schema: String,
    path: PathBuf,
    branch: Option<String>,
    head: String,
    operations: Vec<String>,
    utc: String,
    omissions: Vec<Omission>,
    #[serde(default)]
    backup_bytes: u64,
    #[serde(default)]
    backup_max_bytes: u64,
    #[serde(default)]
    backup_omitted_bytes: u64,
}

#[derive(Default)]
pub(in crate::worktree) struct Receipt {
    pub(in crate::worktree) reference: Option<String>,
    pub(in crate::worktree) reasons: Vec<String>,
    pub(in crate::worktree) bytes: u64,
    pub(in crate::worktree) omitted_bytes: u64,
    pub(in crate::worktree) omissions: Vec<Omission>,
    pub(in crate::worktree) max_bytes: u64,
    pub(in crate::worktree) warnings: Vec<String>,
    pub(in crate::worktree) retention: String,
}

fn failure(message: &str) -> CliError {
    refused("removal-backup-failed", message)
        .with_hint("Target retained; resolve the snapshot error and retry")
}

fn caused_failure(message: &str, cause: CliError) -> CliError {
    let mut error = failure(message);
    error.details = cause
        .details
        .or_else(|| Some(Box::new(json!({"reason": cause.message}))));
    error
}

fn io_failure(message: &str, cause: std::io::Error) -> CliError {
    failure(message).with_details(json!({"reason": format!("local I/O error: {:?}", cause.kind())}))
}

fn indexed(target: &Path, index: &Path, args: &[&str]) -> Result<String, CliError> {
    let mut command = Command::new("git");
    command.args(args).current_dir(target);
    lease::sanitize_git_environment(&mut command);
    command.env("GIT_INDEX_FILE", index);
    let output = lease::removal_probe(&mut command).map_err(|error| {
        failure("bounded snapshot operation failed")
            .with_details(json!({"reason": probe_error_reason(&error)}))
    })?;
    if !output.status.success() {
        return Err(failure("snapshot index operation failed")
            .with_details(json!({"reason":git_error_reason(&output)})));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| failure("snapshot output unreadable"))
}

fn config(target: &Path, key: &str) -> Option<String> {
    git(target, &["config", "--get", key]).ok()
}

pub(in crate::worktree) fn capture(
    target: &Path,
    fence: &Fence,
    acknowledge: bool,
) -> Result<Receipt, CliError> {
    let mut receipt = Receipt {
        max_bytes: 50 * 1024 * 1024,
        ..Receipt::default()
    };
    if let Some(value) = config(target, "worktree.backupMaxBytes") {
        match value.parse::<u64>() {
            Ok(bytes) => receipt.max_bytes = bytes,
            Err(_) => receipt
                .warnings
                .push("invalid worktree.backupMaxBytes; using 50 MiB".into()),
        }
    }
    let (retention, _) = retention(target, &mut receipt.warnings);
    receipt.retention = retention;
    // Ref ancestry includes all local branches, tags and cached remote refs;
    // an unpublished branch already keeps a clean HEAD reachable.
    let reachable = git(
        target,
        &[
            "for-each-ref",
            "--contains",
            &fence.removed_head,
            "--format=%(refname)",
            "refs/heads",
            "refs/tags",
            "refs/remotes",
        ],
    )
    .map_err(|error| caused_failure("HEAD reachability unavailable", error))?;
    let status = git(
        target,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .map_err(|error| caused_failure("working tree inventory unavailable", error))?;
    if !status.is_empty() {
        receipt.reasons.push("dirty-or-untracked".into());
        receipt
            .warnings
            .push("dirty and non-ignored untracked files preserved".into());
    }
    if !fence.operations.is_empty() {
        receipt.reasons.push("git-operation".into());
    }
    if reachable.is_empty() {
        receipt.reasons.push("unreachable-head".into());
    }
    if receipt.reasons.is_empty() {
        return Ok(receipt);
    }
    let output = probe(
        "git",
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        target,
    )
    .map_err(|error| caused_failure("snapshot file inventory unavailable", error))?;
    if !output.status.success() {
        return Err(failure("snapshot file inventory failed")
            .with_details(json!({"reason": git_error_reason(&output)})));
    }
    let mut files = Vec::new();
    for bytes in output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
        let metadata = match fs::symlink_metadata(target.join(&path)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_failure("snapshot file metadata unavailable", error)),
        };
        if metadata.is_file() || metadata.file_type().is_symlink() {
            files.push((path, metadata.len()));
        } else if metadata.is_dir() {
            return Err(failure(
                "snapshot does not preserve nested repositories or submodules",
            ));
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    files.dedup_by(|a, b| a.0 == b.0);
    let total = files
        .iter()
        .try_fold(0u64, |total, (_, bytes)| total.checked_add(*bytes))
        .ok_or_else(|| failure("snapshot size overflow"))?;
    receipt.bytes = total;
    for (path, bytes) in files {
        if receipt.bytes <= receipt.max_bytes {
            break;
        }
        receipt.bytes -= bytes;
        receipt.omitted_bytes += bytes;
        receipt.omissions.push(Omission {
            path: path
                .to_str()
                .ok_or_else(|| failure("oversize snapshot path is not UTF-8"))?
                .to_owned(),
            bytes,
        });
    }
    if !receipt.omissions.is_empty() {
        receipt
            .warnings
            .push("backup size cap omits listed paths; ignored files are never preserved".into());
        if !acknowledge {
            return Err(refused("removal-backup-acknowledgment-required", "backup exceeds size cap; explicit omission acknowledgment required")
                .with_hint("Review backup_omissions; retry with --acknowledge-backup-omissions only if those files are disposable")
                .with_details(json!({"backup_max_bytes":receipt.max_bytes,"backup_bytes":receipt.bytes,"backup_omitted_bytes":receipt.omitted_bytes,"backup_omissions":receipt.omissions,"warnings":receipt.warnings})));
        }
    }
    let temporary = tempfile::tempdir()
        .map_err(|error| io_failure("snapshot temporary directory unavailable", error))?;
    let index = temporary.path().join("index");
    indexed(target, &index, &["read-tree", &fence.removed_head])?;
    let exclusions = receipt
        .omissions
        .iter()
        .map(|omission| format!(":(exclude,literal){}", omission.path))
        .collect::<Vec<_>>();
    let mut add_args = vec!["add", "-A", "--", "."];
    add_args.extend(exclusions.iter().map(String::as_str));
    indexed(target, &index, &add_args)?;
    // Excluded tracked paths retain HEAD content; excluded untracked paths
    // never enter the index. Omitting an edit must not invent a deletion.
    let tree = indexed(target, &index, &["write-tree"])?;
    let utc = time::OffsetDateTime::now_utc();
    let formatted = utc
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| failure("snapshot UTC unavailable"))?;
    let metadata = Metadata {
        schema: "git-cli.worktree-backup.v1".into(),
        path: target.to_path_buf(),
        branch: fence.removed_branch.clone(),
        head: fence.removed_head.clone(),
        operations: fence.operations.clone(),
        utc: formatted,
        omissions: receipt.omissions.clone(),
        backup_bytes: receipt.bytes,
        backup_max_bytes: receipt.max_bytes,
        backup_omitted_bytes: receipt.omitted_bytes,
    };
    let message =
        serde_json::to_string(&metadata).map_err(|_| failure("snapshot metadata unavailable"))?;
    let mut args = vec![
        "commit-tree",
        &tree,
        "-p",
        &fence.removed_head,
        "-m",
        &message,
    ];
    if git(target, &["config", "--bool", "--get", "commit.gpgsign"])
        .is_ok_and(|value| value == "true")
    {
        args.push("-S");
    }
    let commit = git(target, &args)
        .map_err(|error| caused_failure("snapshot commit could not be written", error))?;
    let slug = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| failure("snapshot slug unreadable"))?;
    let reference = format!(
        "refs/worktree-backup/{slug}/{}",
        metadata.utc.replace([':', '.'], "-")
    );
    // Create only: a concurrent snapshot cannot overwrite an existing backup.
    let zero = "0".repeat(commit.len());
    git(target, &["update-ref", &reference, &commit, &zero])
        .map_err(|error| caused_failure("snapshot reference could not be written", error))?;
    receipt.reference = Some(reference);
    Ok(receipt)
}

#[derive(Serialize)]
pub(in crate::worktree) struct RestoreOutput {
    pub(in crate::worktree) path: String,
    pub(in crate::worktree) branch: Option<String>,
    pub(in crate::worktree) backup_ref: String,
    pub(in crate::worktree) parent: String,
}

pub(in crate::worktree) fn restore(
    repo: &Path,
    reference: &str,
    path: Option<&Path>,
) -> Result<RestoreOutput, CliError> {
    if !reference.starts_with("refs/worktree-backup/")
        || !probe("git", &["check-ref-format", reference], repo)?
            .status
            .success()
    {
        return Err(CliError::usage(
            "invalid-backup-ref",
            "restore requires a refs/worktree-backup reference",
        ));
    }
    let message = git(repo, &["show", "-s", "--format=%B", reference])?;
    let metadata: Metadata =
        serde_json::from_str(&message).map_err(|_| failure("backup metadata malformed"))?;
    let parent = git(repo, &["rev-parse", "--verify", &format!("{reference}^")])?;
    if metadata.schema != "git-cli.worktree-backup.v1"
        || metadata.head != parent
        || !metadata.path.is_absolute()
    {
        return Err(failure("backup identity malformed"));
    }
    let target = path.unwrap_or(&metadata.path);
    if target.exists() {
        return Err(CliError::data(
            "worktree-path-exists",
            "restore path already exists",
        ));
    }
    let branch = metadata.branch.filter(|branch| {
        git(
            repo,
            &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
        )
        .is_ok_and(|oid| oid == parent)
    });
    let target_str = target
        .to_str()
        .ok_or_else(|| failure("restore path unreadable"))?;
    let mut args = vec!["worktree", "add"];
    if branch.is_none() {
        args.push("--detach");
    }
    args.extend(["--", target_str, branch.as_deref().unwrap_or(&parent)]);
    git(repo, &args)?;
    let temporary =
        tempfile::tempdir().map_err(|error| io_failure("restore index unavailable", error))?;
    let index = temporary.path().join("index");
    indexed(target, &index, &["read-tree", &parent])?;
    indexed(target, &index, &["read-tree", "--reset", "-u", reference])?;
    Ok(RestoreOutput {
        path: target_str.into(),
        branch,
        backup_ref: reference.into(),
        parent,
    })
}

fn retention(target: &Path, warnings: &mut Vec<String>) -> (String, Option<time::Duration>) {
    let value = config(target, "worktree.backupRetention").unwrap_or_else(|| "30d".into());
    if value == "keep" {
        return (value, None);
    }
    if let Some(seconds) = nils_common::env::parse_duration_seconds(&value)
        .and_then(|seconds| i64::try_from(seconds).ok())
    {
        return (value, Some(time::Duration::seconds(seconds)));
    }
    warnings.push("invalid worktree.backupRetention; backups kept".into());
    ("keep".into(), None)
}

#[derive(Serialize)]
pub(in crate::worktree) struct ExpiryOutput {
    pub(in crate::worktree) expired_refs: Vec<String>,
    pub(in crate::worktree) dry_run: bool,
    retention: String,
    log_path: Option<String>,
    warnings: Vec<String>,
}

pub(in crate::worktree) fn expire(
    repo: &Path,
    dry_run: bool,
    older_than: Option<&str>,
) -> Result<ExpiryOutput, CliError> {
    use std::io::Write;
    use std::os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut warnings = Vec::new();
    let (retention, age) = if let Some(value) = older_than {
        let seconds = nils_common::env::parse_duration_seconds(value)
            .and_then(|s| i64::try_from(s).ok())
            .ok_or_else(|| {
                CliError::usage(
                    "invalid-duration",
                    "--older-than requires a positive duration such as 30d, 1h or 60s",
                )
            })?;
        (value.to_owned(), Some(time::Duration::seconds(seconds)))
    } else {
        retention(repo, &mut warnings)
    };
    let mut result = ExpiryOutput {
        expired_refs: Vec::new(),
        dry_run,
        retention,
        log_path: None,
        warnings,
    };
    let Some(age) = age else { return Ok(result) };
    let now = time::OffsetDateTime::now_utc();
    let rows = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/worktree-backup/",
        ],
    )?;
    let mut candidates = Vec::new();
    for row in rows.lines() {
        let Some((reference, oid)) = row.split_once(' ') else {
            continue;
        };
        let metadata = git(repo, &["show", "-s", "--format=%B", reference])
            .ok()
            .and_then(|message| serde_json::from_str::<Metadata>(&message).ok());
        let Some(metadata) =
            metadata.filter(|metadata| metadata.schema == "git-cli.worktree-backup.v1")
        else {
            result
                .warnings
                .push(format!("backup metadata unavailable; retained {reference}"));
            continue;
        };
        let Ok(created) = time::OffsetDateTime::parse(
            &metadata.utc,
            &time::format_description::well_known::Rfc3339,
        ) else {
            result
                .warnings
                .push(format!("backup time invalid; retained {reference}"));
            continue;
        };
        if now - created >= age {
            candidates.push((reference.to_owned(), oid.to_owned()));
        }
    }
    if dry_run {
        result.expired_refs = candidates.into_iter().map(|row| row.0).collect();
        return Ok(result);
    }
    if candidates.is_empty() {
        return Ok(result);
    }
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let directory = Path::new(&common).join("logs");
    fs::create_dir_all(&directory)
        .map_err(|_| failure("backup expiry log directory unavailable"))?;
    let path = directory.join("worktree-backup-expiry.jsonl");
    let mut log = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| failure("backup expiry log unavailable; references retained"))?;
    let identity = log
        .metadata()
        .map_err(|_| failure("backup expiry log identity unavailable"))?;
    if !identity.is_file()
        || identity.uid() != unsafe { libc::geteuid() }
        || identity.nlink() != 1
        || identity.mode() & 0o077 != 0
        || unsafe { libc::flock(log.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
    {
        return Err(failure(
            "backup expiry log is busy or untrusted; references retained",
        ));
    }
    result.log_path = Some(path.to_string_lossy().into_owned());
    let utc = now
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| failure("expiry UTC unavailable"))?;
    for (reference, oid) in candidates {
        // Persist intent before deletion. A crash leaves a reconcilable log;
        // compare-and-delete never removes a ref replaced since the inventory.
        let record = |state: &str| json!({"schema":"git-cli.worktree-backup-expiry.v1","utc":utc,"backup_ref":reference,"oid":oid,"retention":result.retention,"state":state});
        writeln!(log, "{}", record("pending"))
            .and_then(|_| log.sync_all())
            .map_err(|_| failure("expiry intent could not be logged; reference retained"))?;
        let deleted = git(repo, &["update-ref", "-d", &reference, &oid]);
        writeln!(
            log,
            "{}",
            record(if deleted.is_ok() {
                "expired"
            } else {
                "retained"
            })
        )
        .and_then(|_| log.sync_all())
        .map_err(|_| failure("expiry result log failed; consult pending log entry"))?;
        match deleted {
            Ok(_) => result.expired_refs.push(reference),
            Err(_) => result.warnings.push(format!(
                "backup changed or could not be expired: {reference}"
            )),
        }
    }
    Ok(result)
}

#[derive(Serialize)]
pub(in crate::worktree) struct BackupEntry {
    backup_ref: String,
    commit: String,
    #[serde(flatten)]
    metadata: Metadata,
}

#[derive(Serialize)]
pub(in crate::worktree) struct ListOutput {
    pub(in crate::worktree) backups: Vec<BackupEntry>,
    retention: String,
    warnings: Vec<String>,
}

pub(in crate::worktree) fn list(repo: &Path) -> Result<ListOutput, CliError> {
    let mut warnings = Vec::new();
    let (retention, _) = retention(repo, &mut warnings);
    let rows = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/worktree-backup/",
        ],
    )?;
    let mut backups = Vec::new();
    for row in rows.lines() {
        let Some((reference, oid)) = row.split_once(' ') else {
            continue;
        };
        let metadata = git(repo, &["show", "-s", "--format=%B", reference])
            .ok()
            .and_then(|text| serde_json::from_str::<Metadata>(&text).ok());
        match metadata.filter(|metadata| metadata.schema == "git-cli.worktree-backup.v1") {
            Some(metadata) => backups.push(BackupEntry {
                backup_ref: reference.to_owned(),
                commit: oid.to_owned(),
                metadata,
            }),
            None => warnings.push(format!("backup metadata unavailable: {reference}")),
        }
    }
    Ok(ListOutput {
        backups,
        retention,
        warnings,
    })
}

impl ListOutput {
    pub(in crate::worktree) fn render_text(&self) -> String {
        let mut lines = vec![format!("Backup retention: {}", self.retention)];
        lines.extend(self.backups.iter().map(|entry| {
            format!(
                "{}: {} bytes, {}",
                entry.backup_ref, entry.metadata.backup_bytes, entry.metadata.utc
            )
        }));
        lines.extend(
            self.warnings
                .iter()
                .map(|warning| format!("Warning: {warning}")),
        );
        lines.join("\n")
    }
}

impl ExpiryOutput {
    pub(in crate::worktree) fn render_text(&self) -> String {
        let mut lines = vec![format!(
            "{} backup references {}",
            self.expired_refs.len(),
            if self.dry_run {
                "would expire"
            } else {
                "expired"
            }
        )];
        lines.extend(self.expired_refs.iter().cloned());
        if let Some(path) = &self.log_path {
            lines.push(format!("Expiry log: {path}"));
        }
        lines.extend(
            self.warnings
                .iter()
                .map(|warning| format!("Warning: {warning}")),
        );
        lines.join("\n")
    }
}
