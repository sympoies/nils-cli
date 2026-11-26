//! Host-local admission at the initial durable session creation boundary.
use super::*;
use crate::lifecycle::failed_close::{self, FailedClose};

const OCCUPANCY_LIVENESS_BUDGET: std::time::Duration = std::time::Duration::from_millis(1_500);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    max_active_sessions: Option<u64>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Snapshot {
    max_active_sessions: Option<u64>,
    active_sessions: usize,
}

fn limit(context: &CliContext) -> Result<Option<u64>, CliError> {
    let path = context.state_dir.join("admission.json");
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(config_error()),
    };
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| config_error())?;
    if bytes.len() > 4096 {
        return Err(config_error());
    }
    let settings: Settings = serde_json::from_slice(&bytes).map_err(|_| config_error())?;
    Ok(settings.max_active_sessions)
}

fn config_error() -> CliError {
    CliError::data(
        "session-admission-config-invalid",
        "admission.json must contain an optional non-negative integer max_active_sessions",
        Some(
            json!({"retryable":false,"next_action":"repair-admission-config","recovery":{"kind":"repair_configuration","safe_to_retry":true}}),
        ),
    )
}

/// No registry normalization, record updates, lock-file creation, or runtime
/// launch. Busy lifecycle locks include in-flight creation before tmux exists.
pub(crate) fn snapshot(
    context: &CliContext,
    tmux_bin: Option<&Path>,
) -> Result<(Snapshot, Vec<FailedClose>), CliError> {
    let max_active_sessions = limit(context)?;
    let mut failed_closes = Vec::new();
    let (active_sessions, _) = occupancy(context, tmux_bin, None, Some(&mut failed_closes))?;
    failed_closes.sort_unstable_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok((
        Snapshot {
            max_active_sessions,
            active_sessions,
        },
        failed_closes,
    ))
}

fn occupancy(
    context: &CliContext,
    tmux_bin: Option<&Path>,
    exclude_id: Option<&str>,
    mut failed_closes: Option<&mut Vec<FailedClose>>,
) -> Result<(usize, usize), CliError> {
    let root = context.state_dir.join("sessions");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(_) => return Err(state_error()),
    };
    let tmux = tmux_bin
        .map(Path::to_path_buf)
        .unwrap_or_else(|| resolve_tmux_bin(None));
    let mut candidates = Vec::new();
    let mut active = 0;
    let mut unknown = 0;
    for entry in entries {
        let entry = entry.map_err(|_| state_error())?;
        let kind = entry.file_type().map_err(|_| state_error())?;
        if !kind.is_dir() {
            continue;
        }
        let id = entry.file_name().into_string().map_err(|_| state_error())?;
        validate_id(&id).map_err(|_| state_error())?;
        if exclude_id == Some(id.as_str()) {
            continue;
        }
        // Read the existing lock without creating or chmodding it. A shared
        // nonblocking lock can never wait on a creator holding its lifecycle
        // lock while preparing runtime state. Closing the descriptor unlocks.
        let lock = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(
                context
                    .state_dir
                    .join(SESSION_LOCKS_DIR)
                    .join(format!("{id}.lock")),
            ) {
            Ok(lock) => Some(lock),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Err(state_error()),
        };
        let busy = if let Some(lock) = &lock {
            // SAFETY: this file descriptor stays owned while reading the record.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
                false
            } else if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                true
            } else {
                return Err(state_error());
            }
        } else {
            false
        };
        let path = entry.path().join("session.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                active += usize::from(busy);
                continue;
            }
            Err(_) => return Err(state_error()),
            Ok(_) => {}
        }
        let resolved = ensure_record_in_session_dir(context, &path, &entry.path(), &id)
            .map_err(|_| state_error())?;
        let record = read_session_record(&resolved.record_path).map_err(|_| state_error())?;
        validate_record_id(&record, &id, &resolved.record_path).map_err(|_| state_error())?;
        // A busy stopped record is not yet admitted: resume sets starting
        // under the global guard before releasing it. One-shot records have
        // no startup projection, but hold the same lifecycle lock at launch.
        if busy
            && (record.mode == "run"
                || startup_projection(&record).is_some_and(|startup| startup.state == "starting"))
        {
            active += 1;
            if let Some(failed_closes) = failed_closes.as_deref_mut()
                && let Some(runtime) = &record.runtime
            {
                failed_closes.extend(failed_close::read(
                    context,
                    &id,
                    &runtime.launch_id,
                    runtime.generation,
                ));
            }
            continue;
        }
        // Unknown liveness conservatively occupies a slot. Failed/cancelled
        // creates without a live runtime need no expiry timer or daemon repair.
        if runtime_is_proven_never_launched(&record) || recorded_runtime_is_from_prior_boot(&record)
        {
            continue;
        }
        let journal_target = failed_closes
            .as_ref()
            .and(record.runtime)
            .map(|runtime| (id, runtime.launch_id, runtime.generation));
        candidates.push((record.tmux_session, journal_target));
    }
    // Take the snapshot after examining lifecycle locks. Otherwise an in-flight
    // launch could finish between an early listing and its lock observation.
    if !candidates.is_empty() {
        let sessions = list_tmux_sessions(&tmux);
        for (name, target) in candidates {
            let present = sessions.as_ref().map(|sessions| sessions.contains(&name));
            if present == Some(false) {
                continue;
            }
            active += 1;
            unknown += usize::from(present.is_none());
            if let Some(failed_closes) = failed_closes.as_deref_mut()
                && let Some((id, incarnation, generation)) = target
            {
                failed_closes.extend(failed_close::read(context, &id, &incarnation, generation));
            }
        }
    }
    Ok((active, unknown))
}

fn list_tmux_sessions(tmux: &Path) -> Option<std::collections::BTreeSet<String>> {
    let mut command = ProcessCommand::new(tmux);
    command
        .env("LC_ALL", "C")
        .args(["list-sessions", "-F", "#{session_id}\t#{session_name}"]);
    let output =
        run_output_with_timeout_and_strict_cap(command, OCCUPANCY_LIVENESS_BUDGET, 1024 * 1024)
            .ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        return (tmux_output_reports_absent(&output) && !stderr.contains("can't find session:"))
            .then(std::collections::BTreeSet::new);
    }
    let text = std::str::from_utf8(&output.stdout).ok()?;
    let mut sessions = std::collections::BTreeSet::new();
    for line in text.lines() {
        let (id, name) = line.split_once('\t')?;
        if !id.starts_with('$')
            || id.len() < 2
            || !id[1..].bytes().all(|b| b.is_ascii_digit())
            || name.is_empty()
            || name.contains('\t')
        {
            return None;
        }
        sessions.insert(id.to_string());
        sessions.insert(name.to_string());
    }
    Some(sessions)
}

fn state_error() -> CliError {
    CliError::runtime(
        "session-admission-state-unavailable",
        "session admission occupancy could not be read safely",
        Some(
            json!({"retryable":true,"next_action":"retry-admission","recovery":{"kind":"retry","safe_to_retry":true}}),
        ),
    )
}

/// Serialize the capacity check with initial persistence, never provider or
/// tmux launch. The persisted record plus existing lifecycle lock is the
/// reservation, so independent processes and daemon restarts share capacity.
pub(super) fn reserve(
    context: &CliContext,
    tmux_bin: Option<&Path>,
    exclude_id: Option<&str>,
) -> Result<Option<fs::File>, CliError> {
    let Some(max) = limit(context)? else {
        return Ok(None);
    };
    let lock = coordination::lock_private_store(context, "session-admission.lock")?;
    let (active, unknown) = occupancy(context, tmux_bin, exclude_id, None)?;
    if active as u64 >= max {
        return Err(capacity_error(max, active as u64, unknown as u64));
    }
    Ok(Some(lock))
}

fn capacity_error(max: u64, active: u64, unknown: u64) -> CliError {
    CliError::runtime(
        "session-capacity-exceeded",
        format!(
            "session capacity reached ({active}/{max}); stop or delete an active session before creating another"
        ),
        Some(json!({
            "max_active_sessions":max, "active_sessions":active, "unknown_sessions":unknown,
            "retryable":true, "next_action":"wait-for-capacity",
            "recovery":{"kind":"wait", "condition":"active_sessions_below_limit", "safe_to_retry":true}
        })),
    )
}

/// Preserve only the proven pre-create refusal across Console's two HTTP
/// hops. Reconstruct typed recovery; never forward upstream text or extras.
pub(crate) fn forward_refusal(error: &Value) -> Option<CliError> {
    if error["code"] != "session-capacity-exceeded" {
        return None;
    }
    let details = &error["details"];
    let max = details["max_active_sessions"].as_u64()?;
    let active = details["active_sessions"].as_u64()?;
    let unknown = details["unknown_sessions"].as_u64()?;
    if active < max
        || unknown > active
        || details["retryable"] != true
        || details["recovery"]["safe_to_retry"] != true
    {
        return None;
    }
    Some(capacity_error(max, active, unknown))
}

pub(super) fn create_record_with_lineage(
    request: RecordRequest<'_>,
    initial_lineage: Option<InitialLineage>,
    move_lineage: Option<session_record::ResumeMoveLineage>,
    tmux_bin: Option<&Path>,
) -> Result<CreatedRecord, CliError> {
    let now = Zoned::now();
    let timestamp = now.strftime("%Y%m%d-%H%M%S").to_string();
    let iso = now.timestamp().to_string();
    let title_supplied = request.title.is_some();
    let title = request.title.map(str::to_string);
    let (title, title_state) = match request.title_state {
        Some(title_state) => {
            canonicalize_structured_title_pair(title, title_supplied, title_state)?
        }
        None => (crate::session_title::normalize_manual_title(title)?, None),
    };
    let title_slug = title.as_deref().map(slugify);
    let id = resolve_session_id(
        request.context,
        request.explicit_id,
        request.agent,
        &timestamp,
        title_slug.as_deref(),
    )?;
    let state_root = private_session_state_root(request.context)?;
    let _admission = reserve(request.context, tmux_bin, None)?;
    let lifecycle_lock = acquire_new_session_record_lock(request.context, &state_root, &id)?;
    let tmux_session = format!("hs-{}-{id}", request.agent.as_str());
    let session_dir = session_dir(request.context, &id);
    if session_dir.exists() {
        return Err(CliError::runtime(
            "session-exists",
            format!("session already exists: {id}"),
            Some(json!({ "id": id })),
        ));
    }
    let session_storage = private_session_dir(request.context, &state_root, &session_dir)?;
    if let Err(error) = pause_session_ancestor_for_test("session-dir-created") {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    if let Err(error) = session_storage.validate() {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    if let Err(error) = pause_session_ancestor_for_test("initialization-authority-validated") {
        session_storage.cleanup_if_current();
        return Err(error);
    }

    let prompt_file = match request.prompt {
        Some(prompt) => {
            let path = session_dir.join("prompt.md");
            if let Err(error) = session_storage.write_private_file("prompt.md", prompt.as_bytes()) {
                session_storage.cleanup_if_current();
                return Err(error);
            }
            if let Err(error) = session_storage.validate() {
                session_storage.cleanup_if_current();
                return Err(error);
            }
            Some(path)
        }
        None => None,
    };
    let log_file = request.log_file_name.map(|name| session_dir.join(name));
    let mut record = SessionRecord {
        schema_version: SESSION_DOCUMENT_VERSION.to_string(),
        id,
        agent: request.agent.as_str().to_string(),
        mode: request.mode.to_string(),
        coordination_mode: request.coordination_mode,
        title,
        title_state,
        title_revision: 0,
        cwd: display_path(request.cwd),
        tmux_session: tmux_session.clone(),
        prompt_file: prompt_file.as_ref().map(|path| display_path(path)),
        log_file: log_file.as_ref().map(|path| display_path(path)),
        created_at: iso.clone(),
        updated_at: iso.clone(),
        provider_resume: request.provider_resume,
        runtime: Some(RuntimeInfo {
            kind: "tmux".to_string(),
            tmux_session: tmux_session.clone(),
            generation: 1,
            started_at: now.timestamp().to_string(),
            launch_id: uuid::Uuid::new_v4().to_string(),
            extra: if request.agent == AgentKind::Codex {
                BTreeMap::from([(
                    codex_app_server::ATTENTION_AUTHORITY_KEY.to_string(),
                    json!("hook"),
                )])
            } else {
                BTreeMap::new()
            },
        }),
        public_metadata: None,
        agent_args: request.agent_args,
        agent_bin: request.agent_bin,
        extra: BTreeMap::new(),
        lineage: None,
        work: None,
        lineage_adoption: None,
        role: None,
        resume_sidecar_extra: BTreeMap::new(),
    };
    if let Some(lineage) = move_lineage {
        record.store_resume_move(lineage);
    }
    runtime_environment::store_launch(&mut record);
    origin::Fields::default().store(&mut record);
    if let Some(mut initial) = initial_lineage {
        initial.origin.store(&mut record);
        initial.seed.set_forge_role(initial.role.clone());
        record.lineage = Some(initial.seed.finalize(&record.id, &record.created_at));
        record.work = initial.work;
        record.role = initial.role;
    }
    if request.mode == "interactive" {
        store_startup_projection(&mut record, &starting_projection(&iso, "record"));
    }

    if let Err(error) = lifecycle::begin_creation(request.context, &record) {
        session_storage.cleanup_if_current();
        return Err(error);
    }

    if let Err(error) = write_initial_session_record(&session_storage, &record) {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    if let Err(error) = session_storage.validate() {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    #[cfg(target_os = "linux")]
    let activity_result =
        activity::activate_runtime_in_dir(&session_storage.session_descriptor_path(), &record);
    #[cfg(all(unix, not(target_os = "linux")))]
    let activity_result = activity::activate_new_runtime_with(&record, |name, bytes| {
        session_storage.write_private_file(name, bytes)
    });
    #[cfg(not(unix))]
    let activity_result =
        activity::activate_runtime_in_dir(&session_storage.session_descriptor_path(), &record);
    if let Err(err) = activity_result {
        session_storage.cleanup_if_current();
        return Err(err);
    }
    if let Err(error) = session_storage.validate() {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    #[cfg(target_os = "linux")]
    let coordination_result =
        coordination::prepare_in_dir(&session_storage.session_descriptor_path(), &record);
    #[cfg(all(unix, not(target_os = "linux")))]
    let coordination_result = session_storage.create_private_dir("coordination");
    #[cfg(not(unix))]
    let coordination_result =
        coordination::prepare_in_dir(&session_storage.session_descriptor_path(), &record);
    if let Err(err) = coordination_result {
        session_storage.cleanup_if_current();
        return Err(err);
    }
    if let Err(error) = session_storage.validate() {
        session_storage.cleanup_if_current();
        return Err(error);
    }
    Ok(CreatedRecord {
        record,
        prompt_file,
        session_storage,
        _lifecycle_lock: Some(lifecycle_lock),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::http::{HttpResponse, LoopbackServer};
    use pretty_assertions::assert_eq;

    #[test]
    fn admission_console_relay_retains_capacity_and_not_created_receipt() {
        let temporary = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temporary.path().join("state"),
            host: Some("example-host".into()),
        };
        let id =
            crate::tests::create_test_record_id(&context, AgentKind::Codex, None, Some("caller"));
        let record = load_session_record(&context, &id).unwrap();
        let capability = coordination::provision(&context, &record).unwrap();
        let token = fs::read_to_string(capability).unwrap();
        let incarnation = record.runtime.as_ref().unwrap().launch_id.as_str();
        write_private_file(
            &coordination::heartbeat_path(&context.state_dir, &id),
            format!("{incarnation}:{}\n", Timestamp::now().as_second()).as_bytes(),
        )
        .unwrap();
        let server = LoopbackServer::new().unwrap();
        server.add_route("POST", "/api/coordination/sessions/v1", HttpResponse::new(409, json!({"ok":false,"error":{
            "code":"session-capacity-exceeded", "message":"untrusted-sentinel",
            "details":{"max_active_sessions":1,"active_sessions":1,"unknown_sessions":1,"retryable":true,"recovery":{"safe_to_retry":true},"private_payload":"untrusted-sentinel"}
        }}).to_string()));
        let config = coordination::remote::Config {
            machine: "example-host".into(),
            url: server.url(),
            token: "fixture-relay".into(),
            ingress_token: "fixture-ingress".into(),
        };
        let operation = uuid::Uuid::now_v7().to_string();
        let error = coordination::console_start::relay_route(
            &context,
            Some(&config),
            &id,
            token.trim(),
            &json!({"operation_id":operation,"session":{"agent":"claude","cwd":"/repo"}}),
        )
        .unwrap_err();
        assert_eq!(error.code(), "session-capacity-exceeded");
        assert_eq!(
            server.take_requests().len(),
            1,
            "the real HTTP refusal must arrive"
        );
        let details = error.details().unwrap();
        assert_eq!(details["max_active_sessions"], 1);
        assert_eq!(details["active_sessions"], 1);
        assert_eq!(details["unknown_sessions"], 1);
        assert_eq!(details["retryable"], true);
        assert_eq!(details["next_action"], "wait-for-capacity");
        assert_eq!(details["recovery"]["safe_to_retry"], true);
        assert!(!details.to_string().contains("untrusted-sentinel"));
        assert!(!error.into_inner().message.contains("untrusted-sentinel"));
        let receipt = coordination::console_start::relay_route(
            &context,
            Some(&config),
            &id,
            token.trim(),
            &json!({"operation_id":operation,"reconcile":true}),
        )
        .unwrap();
        assert_eq!(receipt["reconciliation"]["state"], "not_created");
        assert_eq!(receipt["reconciliation"]["safe_to_retry"], true);
        assert!(
            server.take_requests().is_empty(),
            "a known refusal must not submit or probe again"
        );
    }
}
