//! Durable intent for boot recovery. Records lacking intent are never recovered.
use crate::{CliContext, CliError, SessionRecord, write_session_record};
use serde_json::json;

const KEY: &str = "boot_recovery";
const SHUTDOWN_MARK_TTL_SECONDS: i64 = 24 * 60 * 60;

pub(crate) fn shutdown_mark_armed(record: &SessionRecord) -> bool {
    record.extra.get(KEY).is_some_and(|value| {
        value.get("shutdown_boot_id").is_some() || value.get("marked_at_epoch").is_some()
    })
}

pub(crate) fn persist_launch(
    context: &CliContext,
    record: &mut SessionRecord,
    identity: &crate::TmuxRuntimeIdentity,
) -> Result<(), CliError> {
    crate::persist_tmux_runtime_identity(record, identity).map_err(|_| {
        CliError::runtime(
            "tmux-runtime-identity-persist-failed",
            "failed to retain the launched tmux runtime identity",
            Some(json!({"id":record.id})),
        )
    })?;
    if let Some(entry) = record.extra.get_mut(KEY) {
        clear_mark(entry);
    }
    write_session_record(context, record)
}

pub(crate) fn mark_stopped(
    context: &CliContext,
    record: &mut SessionRecord,
) -> Result<(), CliError> {
    let Some(entry) = record
        .extra
        .get_mut(KEY)
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    let fields_before = entry.len();
    entry.retain(|key, _| {
        !matches!(
            key.as_str(),
            "shutdown_boot_id" | "marked_at_epoch" | "receipt" | "receipt_boot_id"
        )
    });
    if entry.len() == fields_before {
        return Ok(());
    }
    write_session_record(context, record)
}

/// Called while the long-lived serve process owns the state root. Only lock
/// timeouts are retried; malformed or unwritable records are reported once.
pub(crate) fn recover(
    context: &CliContext,
    tmux: &std::path::Path,
    retry_ids: Option<Vec<String>>,
) -> Result<Vec<String>, CliError> {
    let mut ids = match retry_ids {
        Some(ids) => ids,
        None => session_ids(context, "boot-recovery-list-failed")?,
    };
    ids.sort();
    let boot = current_boot_id().ok_or_else(|| {
        CliError::runtime(
            "boot-id-unavailable",
            "cannot verify the current boot",
            None,
        )
    })?;
    let disabled = std::env::var("AGENT_SESSION_BOOT_RECOVERY").is_ok_and(|value| value == "0");
    let excluded = std::env::var("AGENT_SESSION_BOOT_RECOVERY_EXCLUDE").unwrap_or_default();
    let mut pending = Vec::new();
    for id in ids {
        let lock = match crate::acquire_session_record_lock_timed(
            context,
            &id,
            std::time::Duration::from_secs(1),
        ) {
            Ok(lock) => lock,
            Err(error) if error.code() == "session-record-lock-timeout" => {
                eprintln!("warning: boot recovery for {id} deferred: {}", error.code());
                pending.push(id);
                continue;
            }
            Err(error) => {
                eprintln!("warning: boot recovery for {id} failed: {}", error.code());
                continue;
            }
        };
        let result = (|| -> Result<(), CliError> {
            let Some(mut record) = read_exact(context, &id)? else {
                return Ok(());
            };
            let mark = record
                .extra
                .get(KEY)
                .and_then(|value| value.get("shutdown_boot_id"))
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let age = record
                .extra
                .get(KEY)
                .and_then(|value| value.get("marked_at_epoch"))
                .and_then(|value| value.as_i64())
                .and_then(|at| jiff::Timestamp::now().as_second().checked_sub(at))
                .filter(|age| *age >= 0);
            let expired = mark.is_some() && age.is_some_and(|age| age >= SHUTDOWN_MARK_TTL_SECONDS);
            if mark.as_deref() == Some(boot.as_str()) && age.is_some() && !expired {
                return Ok(());
            }
            let previous = crate::recorded_runtime_is_from_prior_boot(&record);
            let processed_this_boot = record
                .extra
                .get(KEY)
                .and_then(|value| value.get("receipt_boot_id"))
                .and_then(|value| value.as_str())
                == Some(boot.as_str());
            if mark.is_none() && (!previous || processed_this_boot) {
                return Ok(());
            }
            let reason = if expired {
                Some("shutdown-intent-expired")
            } else if mark.is_none() {
                Some("no-shutdown-intent")
            } else if age.is_none()
                || !previous
                || mark.as_deref() != prior_boot_id(&record).as_deref()
            {
                Some("shutdown-intent-unverified")
            } else if disabled {
                Some("host-opt-out")
            } else if excluded.split(',').any(|item| item.trim() == id) {
                Some("session-opt-out")
            } else if record.provider_resume.is_none() {
                Some("resume-metadata-missing")
            } else {
                None
            };
            if let Some(value) = record.extra.get_mut(KEY) {
                clear_mark(value);
            }
            if let Some(reason) = reason {
                receipt(
                    &mut record,
                    json!({"state":"skipped", "reason":reason}),
                    &boot,
                );
                return write_session_record(context, &record);
            }
            // Consume before launching so a crash cannot replay this authorization.
            receipt(
                &mut record,
                json!({"state":"failed", "error":{"code":"recovery-interrupted"}}),
                &boot,
            );
            write_session_record(context, &record)?;
            wait_for_previous_heartbeat(context, &record);
            match crate::resume_session_locked(context, record, tmux) {
                Ok(outcome) => {
                    let saved = read_exact(context, &id).and_then(|current| {
                        let mut current = current.ok_or_else(|| {
                            CliError::runtime(
                                "session-not-found",
                                "session disappeared during recovery",
                                None,
                            )
                        })?;
                        receipt(&mut current, json!({"state":"resumed"}), &boot);
                        write_session_record(context, &current)
                    });
                    crate::restart::release_bootstrap(outcome.bootstrap, lock);
                    saved
                }
                Err(error) => {
                    let mut current = read_exact(context, &id)?.ok_or_else(|| {
                        CliError::runtime(
                            "session-not-found",
                            "session disappeared during recovery",
                            None,
                        )
                    })?;
                    receipt(
                        &mut current,
                        json!({"state":"failed", "error":{"code":error.code()}}),
                        &boot,
                    );
                    write_session_record(context, &current)
                }
            }
        })();
        if let Err(error) = result {
            eprintln!("warning: boot recovery for {id} failed: {}", error.code());
        }
    }
    Ok(pending)
}

fn session_ids(context: &CliContext, code: &'static str) -> Result<Vec<String>, CliError> {
    let entries = match std::fs::read_dir(context.state_dir.join("sessions")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(CliError::runtime(
                code,
                "cannot enumerate session records",
                None,
            ));
        }
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|_| CliError::runtime(code, "cannot enumerate session records", None))?;
        if !entry
            .file_type()
            .map_err(|_| CliError::runtime(code, "cannot inspect session entry", None))?
            .is_dir()
        {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if crate::validate_id(&id).is_ok() {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}

// A prior-boot heartbeat cannot renew, but the shared resume guard deliberately
// treats it as live for 30 seconds. Preserve that guard and wait only long
// enough for an ordinary old heartbeat to become stale.
fn wait_for_previous_heartbeat(context: &CliContext, record: &SessionRecord) {
    let Some(incarnation) = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
    else {
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(32);
    while crate::coordination::broker::heartbeat_fresh(context, &record.id, incarnation, 0)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn read_exact(context: &CliContext, id: &str) -> Result<Option<SessionRecord>, CliError> {
    let directory = crate::session_dir(context, id);
    let path = directory.join("session.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(crate::session_io_error("session-read-failed", &path, error)),
        Ok(_) => {}
    }
    let resolved = crate::ensure_record_in_session_dir(context, &path, &directory, id)?;
    let record = crate::read_session_record(&resolved.record_path)?;
    crate::validate_record_id(&record, id, &resolved.record_path)?;
    Ok(Some(record))
}

fn clear_mark(value: &mut serde_json::Value) {
    if let Some(entry) = value.as_object_mut() {
        entry.remove("shutdown_boot_id");
        entry.remove("marked_at_epoch");
    }
}

fn receipt(record: &mut SessionRecord, value: serde_json::Value, boot: &str) {
    let entry = record.extra.entry(KEY.into()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    entry["receipt"] = value;
    entry["receipt_boot_id"] = json!(boot);
}

/// Public projection contains only the last recovery result.
pub(crate) fn projection(record: &SessionRecord) -> Option<serde_json::Value> {
    let receipt = record.extra.get(KEY)?.get("receipt")?;
    match receipt.get("state")?.as_str()? {
        "resumed" => Some(json!({"state":"resumed"})),
        "skipped" => {
            let reason = receipt.get("reason")?.as_str()?;
            if !matches!(
                reason,
                "host-opt-out"
                    | "session-opt-out"
                    | "no-shutdown-intent"
                    | "shutdown-intent-unverified"
                    | "shutdown-intent-expired"
                    | "resume-metadata-missing"
            ) {
                return None;
            }
            Some(json!({"state":"skipped", "reason":reason}))
        }
        "failed" => {
            let code = receipt.get("error")?.get("code")?.as_str()?;
            if code.len() > 128
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            {
                return None;
            }
            Some(json!({"state":"failed", "error":{"code":code}}))
        }
        _ => None,
    }
}

#[derive(Debug, clap::Args)]
pub struct PrepareShutdownArgs {
    /// Override the tmux executable.
    #[arg(long)]
    tmux_bin: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value_t = nils_common::cli_contract::OutputFormat::Text)]
    pub(crate) format: nils_common::cli_contract::OutputFormat,
}

pub(crate) fn run_prepare(context: &CliContext, args: PrepareShutdownArgs) -> i32 {
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    match prepare(context, &tmux) {
        Ok(value) => {
            crate::render_single_success("prepare-shutdown", args.format, &value, |value| {
                format!("{}\n", value)
            })
        }
        Err(error) => crate::render_error("prepare-shutdown", args.format, error),
    }
}

fn current_boot_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        crate::linux_boot_id().ok()
    }
    #[cfg(target_os = "macos")]
    {
        crate::capture_macos_boot_id()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn prior_boot_id(record: &SessionRecord) -> Option<String> {
    let identity = crate::persisted_tmux_runtime_identity(record)
        .ok()
        .flatten()?;
    identity
        .pid_namespace
        .map(|namespace| namespace.boot_id)
        .or_else(|| identity.control_group.and_then(|group| group.boot_id))
        .or(identity.macos_boot_id)
}

fn prepare(context: &CliContext, tmux: &std::path::Path) -> Result<serde_json::Value, CliError> {
    let boot_id = current_boot_id().ok_or_else(|| {
        CliError::runtime(
            "boot-id-unavailable",
            "cannot verify the current boot",
            None,
        )
    })?;
    let ids = session_ids(context, "shutdown-mark-list-failed")?;
    let mut rows = Vec::new();
    let mut failed = false;
    for id in ids {
        let result = (|| -> Result<Option<serde_json::Value>, CliError> {
            let _lock = crate::acquire_session_record_lock_timed(
                context,
                &id,
                std::time::Duration::from_secs(1),
            )?;
            let Some(mut record) = read_exact(context, &id)? else {
                return Ok(None);
            };
            crate::restart::ensure_not_fenced(context, &record)?;
            // An interrupted or repeated preparation cannot leave an older mark armed.
            if let Some(entry) = record
                .extra
                .get_mut(KEY)
                .and_then(|value| value.as_object_mut())
            {
                entry.remove("shutdown_boot_id");
                entry.remove("marked_at_epoch");
                write_session_record(context, &record)?;
            }
            let reason = if crate::recorded_runtime_is_from_prior_boot(&record) {
                Some("prior-boot")
            } else if prior_boot_id(&record).as_deref() != Some(boot_id.as_str()) {
                Some("boot-identity-unverified")
            } else {
                match (
                    crate::persisted_tmux_runtime_identity(&record),
                    crate::capture_tmux_runtime_identity(
                        context,
                        &record,
                        tmux,
                        std::time::Duration::from_secs(2),
                    ),
                ) {
                    (Ok(Some(saved)), Ok(crate::TmuxRuntimeProbe::Running(live)))
                        if saved.same_runtime_target(&live)
                            && saved.same_process_identity(&live) =>
                    {
                        None
                    }
                    (Ok(_), Ok(crate::TmuxRuntimeProbe::Stopped)) => Some("stopped"),
                    _ => Some("runtime-identity-unverified"),
                }
            };
            if reason.is_none() {
                let value = record.extra.entry(KEY.into()).or_insert_with(|| json!({}));
                if !value.is_object() {
                    *value = json!({});
                }
                value["shutdown_boot_id"] = json!(boot_id);
                value["marked_at_epoch"] = json!(jiff::Timestamp::now().as_second());
                write_session_record(context, &record)?;
            }
            Ok(Some(if let Some(reason) = reason {
                json!({"id":id,"state":"skipped","reason":reason})
            } else {
                json!({"id":id,"state":"marked"})
            }))
        })();
        match result {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(error) => {
                failed = true;
                rows.push(json!({"id":id,"state":"failed","error":{"code":error.code()}}));
            }
        }
    }
    if failed {
        return Err(CliError::runtime(
            "shutdown-mark-incomplete",
            "one or more session marks failed",
            Some(json!({"records":rows})),
        ));
    }
    Ok(json!({"records":rows}))
}
