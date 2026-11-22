//! The tmp-class owner: fail closed on uncertain identity, ownership or liveness.
use super::*;
use std::collections::BTreeMap;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

mod filesystem;

#[derive(Debug, Serialize)]
pub(crate) struct SweepReport {
    pub(crate) grace_secs: u64,
    pub(crate) removed: usize,
    pub(crate) retained: BTreeMap<&'static str, usize>,
}

pub(super) fn reclaim_for_launch(root: &Path, id: &str) -> io::Result<()> {
    let boundary = filesystem::Boundary::open(root)?;
    if !boundary.trusted_tree(id) {
        return Err(io::Error::other("session tmp tree is untrusted"));
    }
    boundary.remove_tree(id)
}

impl SweepReport {
    fn keep(&mut self, reason: &'static str) {
        *self.retained.entry(reason).or_default() += 1;
    }
}

pub(super) fn owned_metadata(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    // SAFETY: geteuid has no preconditions.
    (metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o022 == 0
        && !metadata.file_type().is_symlink())
    .then_some(metadata)
}

pub(super) fn read_owned_owner(dir: &Path, id: &str) -> Option<Owner> {
    if !owned_metadata(dir)?.is_dir() {
        return None;
    }
    let path = dir.join(OWNER_FILE);
    let metadata = owned_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let owner: Owner = serde_json::from_reader(file).ok()?;
    (owner.schema_version == OWNER_SCHEMA && owner.session_id == id).then_some(owner)
}

#[cfg(not(target_os = "macos"))]
fn stopped(context: &CliContext, tmux: &Path, record: &SessionRecord) -> &'static str {
    match session_status(context, tmux, record).as_str() {
        "running" => "live",
        "stopped" => match persisted_tmux_runtime_identity(record) {
            Ok(Some(_)) => match coordination_runtime_evidence(context, record) {
                Ok(evidence) => match evidence.status {
                    CoordinationRuntimeStatus::Stopped => "stopped",
                    CoordinationRuntimeStatus::Running => "live",
                    CoordinationRuntimeStatus::Unknown => "unknown",
                },
                Err(_) => "unknown",
            },
            Ok(None)
                if record
                    .runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.kind == "tmux") =>
            {
                "stopped"
            }
            Ok(None) | Err(_) => "unknown",
        },
        _ => "unknown",
    }
}

#[cfg(target_os = "macos")]
fn stopped(context: &CliContext, tmux: &Path, record: &SessionRecord) -> &'static str {
    stopped_macos(
        context,
        tmux,
        record,
        crate::capture_macos_boot_id().as_deref(),
    )
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn stopped_macos(
    context: &CliContext,
    tmux: &Path,
    record: &SessionRecord,
    boot: Option<&str>,
) -> &'static str {
    let identity = persisted_tmux_runtime_identity(record).ok().flatten();
    if identity.as_ref().is_some_and(|identity| {
        crate::macos_prior_boot_proven(identity.macos_boot_id.as_deref(), boot)
    }) {
        return "stopped";
    }
    match crate::live_status(tmux, &record.tmux_session).as_str() {
        "running" => return "live",
        "stopped" => {}
        _ => return "unknown",
    }
    let Some((identity, boot)) = identity.as_ref().zip(boot) else {
        return "unknown";
    };
    if identity
        .process_group_id
        .map(crate::process_group_status)
        .map(crate::conservative_coordination_process_group_status)
        == Some(crate::ProcessGroupStatus::Running)
    {
        return "live";
    }
    match crate::coordination::broker::macos_tmp_runtime_status(
        context, tmux, record, identity, boot,
    ) {
        CoordinationRuntimeStatus::Stopped => "stopped",
        CoordinationRuntimeStatus::Running => "live",
        CoordinationRuntimeStatus::Unknown => "unknown",
    }
}

/// Explicit calls and daemon maintenance walk the entire root. Launch callers
/// retain their short best-effort budget; daemon maintenance provides progress.
pub(crate) fn sweep(
    context: &CliContext,
    tmux_bin: &Path,
    grace: Duration,
    budget: Option<Duration>,
) -> Result<SweepReport, CliError> {
    sweep_with_runtime_check(context, tmux_bin, grace, budget, stopped)
}

pub(super) fn sweep_with_runtime_check(
    context: &CliContext,
    tmux_bin: &Path,
    grace: Duration,
    budget: Option<Duration>,
    runtime_check: impl Fn(&CliContext, &Path, &SessionRecord) -> &'static str,
) -> Result<SweepReport, CliError> {
    let root = tmp_root(&context.state_dir)?;
    let mut report = SweepReport {
        grace_secs: grace.as_secs(),
        removed: 0,
        retained: BTreeMap::new(),
    };
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Ok(_) if owned_metadata(&root).is_some_and(|metadata| metadata.is_dir()) => {}
        _ => {
            return Err(CliError::runtime(
                "session-tmpdir-root-untrusted",
                "session tmp root is unavailable or untrusted",
                None,
            ));
        }
    }
    let boundary = filesystem::Boundary::open(&root).map_err(|_| {
        CliError::runtime(
            "session-tmpdir-root-untrusted",
            "session tmp mount boundary could not be verified",
            None,
        )
    })?;
    let entries = fs::read_dir(&root).map_err(|_| {
        CliError::runtime(
            "session-tmpdir-list-failed",
            "session tmp root could not be enumerated",
            None,
        )
    })?;
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| validate_id(name).is_ok())
        .collect();
    ids.sort();
    let started = Instant::now();
    for id in ids {
        if budget.is_some_and(|budget| started.elapsed() >= budget) {
            break;
        }
        let dir = root.join(&id);
        let Some(mut owner) = read_owned_owner(&dir, &id) else {
            report.keep("untrusted");
            continue;
        };
        // A missing foreign root is not authority to reclaim its directories.
        if owner.state_dir != display_path(&context.state_dir) {
            report.keep("foreign");
            continue;
        }
        // Creation, resume and deletion use this same lock. Hold it through the
        // final ownership check and removal, including the missing-record case.
        let Ok(Some(_lock)) = try_acquire_session_record_lock(context, &id) else {
            report.keep("busy");
            continue;
        };
        let due = match fs::symlink_metadata(session_dir(context, &id).join("session.json")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::metadata(&dir)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= grace),
            Ok(metadata) if metadata.is_file() => {
                let Ok(record) = load_session_record(context, &id) else {
                    report.keep("unreadable");
                    continue;
                };
                let status = runtime_check(context, tmux_bin, &record);
                if status != "stopped" {
                    report.keep(status);
                    continue;
                }
                if owner.stopped_at_epoch.is_none() {
                    owner.stopped_at_epoch = Some(now_epoch());
                    if write_owner(&dir, &owner).is_err() {
                        report.keep("unreadable");
                        continue;
                    }
                }
                owner.stopped_at_epoch.is_some_and(|at| expired(at, grace))
            }
            _ => {
                report.keep("unreadable");
                continue;
            }
        };
        if !due {
            report.keep("grace");
        } else if !boundary.trusted_tree(&id)
            || !owner_of(&dir, &id, &context.state_dir)
                .is_some_and(|current| current.stopped_at_epoch == owner.stopped_at_epoch)
        {
            report.keep("untrusted");
        } else if boundary.remove_tree(&id).is_ok() {
            report.removed += 1;
        } else {
            report.keep("remove-failed");
        }
    }
    Ok(report)
}
