//! Timing for one admitted launch. Provider observations carry only identity and time.
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{CliContext, CliError, SessionRecord, session_dir};

const RECEIPT_FILE: &str = "resume-metrics.json";
const MAX_RECEIPT_BYTES: u64 = 4096;
pub(crate) const WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LaunchKind {
    Initial,
    Resume,
}

impl LaunchKind {
    pub(crate) fn for_restore(restoring: bool) -> Self {
        if restoring {
            Self::Resume
        } else {
            Self::Initial
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Resume => "resume",
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ResumeMetrics {
    kind: &'static str,
    outcome: &'static str,
    started_at: Option<String>,
    completed_at: Option<String>,
    deadline_at: Option<String>,
    duration_ms: Option<u64>,
    unknown_reason: Option<&'static str>,
}

impl ResumeMetrics {
    pub(crate) fn unknown(reason: &'static str) -> Self {
        Self {
            kind: "unknown",
            outcome: "unknown",
            started_at: None,
            completed_at: None,
            deadline_at: None,
            duration_ms: None,
            unknown_reason: Some(reason),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sample {
    at: String,
    tick_ns: Option<u64>,
}

impl Sample {
    pub(crate) fn now() -> Self {
        Self {
            at: Timestamp::now().to_string(),
            tick_ns: monotonic_tick(),
        }
    }

    fn timestamp(&self) -> Option<Timestamp> {
        self.at.parse().ok()
    }
}

fn monotonic_tick() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time points to the live timespec that clock_gettime initializes.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    u64::try_from(time.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.tv_nsec).ok()?)
}

fn boot_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        crate::linux_boot_id().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        crate::capture_macos_boot_id()
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinishOutcome {
    Succeeded,
    ProviderFailed,
    LaunchFailed,
    UnsupportedRuntime,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    outcome: FinishOutcome,
    sample: Option<Sample>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    binding: String,
    kind: LaunchKind,
    boot_id: Option<String>,
    start: Sample,
    finish: Option<Finish>,
}

impl Receipt {
    fn elapsed(&self, sample: &Sample, boot: Option<&str>) -> Option<Duration> {
        if boot.is_none() || self.boot_id.as_deref() != boot {
            return None;
        }
        let nanos = sample.tick_ns?.checked_sub(self.start.tick_ns?)?;
        Some(Duration::from_nanos(nanos))
    }

    fn accepts(&self, sample: &Sample, boot: Option<&str>) -> bool {
        self.kind == LaunchKind::Resume
            && self.finish.is_none()
            && self.elapsed(sample, boot).is_some_and(within_window)
    }

    fn projection(&self, now: &Sample, boot: Option<&str>) -> ResumeMetrics {
        let mut view = ResumeMetrics::unknown("clock_unavailable");
        view.kind = self.kind.name();
        view.started_at = self.start.timestamp().map(|at| at.to_string());
        if let Some(finish) = self.finish.as_ref() {
            match finish.outcome {
                FinishOutcome::LaunchFailed => {
                    view.outcome = "failed";
                    view.unknown_reason = Some("launch_failed");
                    view.completed_at = finish
                        .sample
                        .as_ref()
                        .and_then(Sample::timestamp)
                        .map(|at| at.to_string());
                    return view;
                }
                FinishOutcome::UnsupportedRuntime => {
                    view.unknown_reason = Some("unsupported_runtime");
                    return view;
                }
                _ => {}
            }
        }
        if self.kind == LaunchKind::Initial {
            view.outcome = "not_applicable";
            view.unknown_reason = Some("not_applicable");
            return view;
        }
        view.deadline_at = self
            .start
            .timestamp()
            .and_then(|at| {
                at.checked_add(SignedDuration::from_secs(WINDOW.as_secs() as i64))
                    .ok()
            })
            .map(|at| at.to_string());
        let Some(elapsed) = self.elapsed(now, boot) else {
            return view;
        };
        match self.finish.as_ref() {
            Some(Finish {
                outcome,
                sample: Some(end),
            }) => {
                let Some(duration) = self
                    .elapsed(end, boot)
                    .filter(|delta| within_window(*delta) && *delta <= elapsed)
                else {
                    return view;
                };
                view.completed_at = end.timestamp().map(|at| at.to_string());
                if *outcome == FinishOutcome::Succeeded {
                    view.outcome = "succeeded";
                    view.unknown_reason = None;
                    view.duration_ms = u64::try_from(duration.as_millis()).ok();
                } else {
                    view.outcome = "failed";
                    view.unknown_reason = Some("provider_failed");
                }
            }
            None if within_window(elapsed) => {
                view.outcome = "pending";
                view.unknown_reason = Some("pending");
            }
            None => {
                view.outcome = "timed_out";
                view.unknown_reason = Some("timeout");
            }
            _ => {}
        }
        view
    }
}

fn within_window(elapsed: Duration) -> bool {
    elapsed < WINDOW
}

fn binding(record: &SessionRecord, kind: LaunchKind) -> Option<String> {
    let runtime = record
        .runtime
        .as_ref()
        .filter(|r| !r.launch_id.is_empty())?;
    let conversation = match kind {
        LaunchKind::Initial => None,
        LaunchKind::Resume => {
            let resume = record
                .provider_resume
                .as_ref()
                .filter(|p| p.provider == record.agent && !p.session_id.trim().is_empty())?;
            Some((&resume.provider, &resume.session_id))
        }
    };
    let inputs = serde_json::to_vec(&(
        &record.id,
        &record.created_at,
        &record.agent,
        &runtime.kind,
        &runtime.launch_id,
        runtime.generation,
        conversation,
        crate::session_provider_config_dir(record),
        crate::session_effective_provider_config_dir(record),
        crate::session_agent_profile(record),
    ))
    .ok()?;
    Some(crate::coordination::digest_bytes(&inputs))
}

fn matches(context: &CliContext, record: &SessionRecord, receipt: &Receipt) -> bool {
    binding(record, receipt.kind).as_deref() == Some(receipt.binding.as_str())
        && (receipt.kind == LaunchKind::Initial
            || crate::conversation::binding_is_current(context, record))
}

/// Pin the session directory so a late writer cannot publish into a replacement.
struct Storage {
    directory: File,
    path: PathBuf,
}

impl Storage {
    fn open(context: &CliContext, record: &SessionRecord) -> Option<Self> {
        let path = session_dir(context, &record.id);
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .ok()?;
        crate::validate_session_ancestor_path_identity(&path, &directory, "session").ok()?;
        Some(Self { directory, path })
    }

    fn lock(context: &CliContext, record: &SessionRecord) -> Option<Self> {
        let storage = Self::open(context, record)?;
        // SAFETY: the borrowed directory descriptor remains live until the guard drops.
        let locked =
            unsafe { libc::flock(storage.directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        (locked == 0).then_some(storage)
    }

    fn read(&self) -> Option<Receipt> {
        // SAFETY: openat borrows the directory descriptor and a static C string.
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                c"resume-metrics.json".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return None;
        }
        // SAFETY: openat returned a fresh descriptor owned by this call.
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().ok()?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.len() > MAX_RECEIPT_BYTES
        {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECEIPT_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_RECEIPT_BYTES {
            return None;
        }
        serde_json::from_slice(&bytes).ok()
    }

    fn write(&self, receipt: &Receipt) -> Option<()> {
        let bytes = serde_json::to_vec(receipt).ok()?;
        crate::write_private_file_at(
            &self.directory,
            &self.path.join(RECEIPT_FILE),
            RECEIPT_FILE,
            &bytes,
        )
        .ok()
    }
}

fn begin(context: &CliContext, record: &SessionRecord, kind: LaunchKind) -> Option<()> {
    if !matches!(record.agent.as_str(), "claude" | "codex") {
        return None;
    }
    let binding = binding(record, kind)?;
    let storage = Storage::lock(context, record)?;
    if storage
        .read()
        .is_some_and(|previous| previous.binding == binding)
    {
        return None;
    }
    let finish = (kind == LaunchKind::Resume
        && record.agent == "codex"
        && !crate::codex_app_server::runtime_is_supported(record))
    .then_some(Finish {
        outcome: FinishOutcome::UnsupportedRuntime,
        sample: None,
    });
    let receipt = Receipt {
        binding,
        kind,
        boot_id: boot_id(),
        start: Sample::now(),
        finish,
    };
    storage.write(&receipt)
}

pub(super) fn release_gate(
    context: &CliContext,
    record: &SessionRecord,
    kind: LaunchKind,
    gate: &Path,
) -> Result<(), CliError> {
    // Make the start visible before the held provider can report restoration.
    let attempt = begin(context, record, kind);
    let result = crate::write_private_file(gate, b"ready\n");
    if result.is_err()
        && attempt.is_some()
        && let Some(storage) = Storage::lock(context, record)
        && let Some(mut receipt) = storage.read()
        && matches(context, record, &receipt)
    {
        receipt.finish = Some(Finish {
            outcome: FinishOutcome::LaunchFailed,
            sample: Some(Sample::now()),
        });
        let _ = storage.write(&receipt);
    }
    result
}

pub(crate) fn projection(context: &CliContext, record: &SessionRecord) -> ResumeMetrics {
    let Some(receipt) = Storage::open(context, record).and_then(|storage| storage.read()) else {
        return ResumeMetrics::unknown("not_recorded");
    };
    if !matches(context, record, &receipt) {
        return ResumeMetrics::unknown("identity_changed");
    }
    receipt.projection(&Sample::now(), boot_id().as_deref())
}

pub(crate) fn pending(context: &CliContext, record: &SessionRecord) -> bool {
    Storage::open(context, record)
        .and_then(|storage| storage.read())
        .is_some_and(|receipt| {
            receipt.accepts(&Sample::now(), boot_id().as_deref())
                && matches(context, record, &receipt)
        })
}

pub(crate) fn finish(
    context: &CliContext,
    record: &SessionRecord,
    outcome: FinishOutcome,
    observed: Sample,
) {
    let Some(storage) = Storage::lock(context, record) else {
        return;
    };
    let Some(mut receipt) = storage.read() else {
        return;
    };
    if !receipt.accepts(&observed, boot_id().as_deref())
        || binding(record, receipt.kind).as_deref() != Some(receipt.binding.as_str())
    {
        return;
    }
    let Ok(current) = crate::load_session_record(context, &record.id) else {
        return;
    };
    if !matches(context, &current, &receipt) {
        return;
    }
    receipt.finish = Some(Finish {
        outcome,
        sample: Some(observed),
    });
    let _ = storage.write(&receipt);
}

#[cfg(test)]
mod tests;
