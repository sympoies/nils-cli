//! Session list traversal; CLI-only request-local usage discovery.
use super::*;

pub(super) fn list_sessions(
    context: &CliContext,
    tmux_bin: Option<&Path>,
) -> Result<Vec<SessionView>, CliError> {
    list_sessions_with_shadow_sampling(context, tmux_bin, false, true)
}

pub(super) fn list_sessions_for_serve(
    context: &CliContext,
    tmux_bin: Option<&Path>,
) -> Result<Vec<SessionView>, CliError> {
    list_sessions_with_shadow_sampling(context, tmux_bin, true, false)
}

fn list_sessions_with_shadow_sampling(
    context: &CliContext,
    tmux_bin: Option<&Path>,
    schedule_shadow_sampling: bool,
    query_scoped_usage: bool,
) -> Result<Vec<SessionView>, CliError> {
    let mut records = Vec::new();
    let mut query = query_scoped_usage.then(context_usage::Query::default);
    visit_with_usage(
        context,
        tmux_bin,
        schedule_shadow_sampling,
        query.as_mut(),
        &mut |view| {
            records.push(view?);
            Ok(())
        },
    )?;
    records.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(records)
}

/// Build one view per session directory, in directory order, handing each
/// per-entry result to `visit`. `list` stops at the first failed entry; the
/// session board (`board.rs`) instead skips and counts it. An error from the
/// sessions root itself, or one `visit` returns, ends the walk.
pub(super) fn visit_session_views(
    context: &CliContext,
    tmux_bin: Option<&Path>,
    schedule_shadow_sampling: bool,
    visit: &mut dyn FnMut(Result<SessionView, CliError>) -> Result<(), CliError>,
) -> Result<(), CliError> {
    visit_with_usage(context, tmux_bin, schedule_shadow_sampling, None, visit)
}

fn visit_with_usage(
    context: &CliContext,
    tmux_bin: Option<&Path>,
    schedule_shadow_sampling: bool,
    mut query: Option<&mut context_usage::Query>,
    visit: &mut dyn FnMut(Result<SessionView, CliError>) -> Result<(), CliError>,
) -> Result<(), CliError> {
    let sessions_root = context.state_dir.join("sessions");
    if !sessions_root.exists() {
        return Ok(());
    }
    let tmux_bin = tmux_bin
        .map(Path::to_path_buf)
        .unwrap_or_else(|| resolve_tmux_bin(None));
    let tmux_snapshots = tmux_session_snapshots(&tmux_bin);
    let coordination_registry = coordination::public_summary_registry_snapshot(context);
    for entry in fs::read_dir(&sessions_root).map_err(|err| {
        CliError::runtime(
            "session-list-failed",
            format!("failed to read {}: {err}", sessions_root.display()),
            Some(json!({ "path": display_path(&sessions_root) })),
        )
    })? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                visit(Err(CliError::runtime(
                    "session-list-failed",
                    format!("failed to read session entry: {err}"),
                    None,
                )))?;
                continue;
            }
        };
        if entry.path().is_dir() {
            let entry_name = entry.file_name().to_string_lossy().to_string();
            let record_path = entry.path().join("session.json");
            if record_path.is_file() {
                let record =
                    ensure_record_in_session_dir(context, &record_path, &entry.path(), &entry_name)
                        .and_then(|resolved| {
                            let record = read_session_record(&resolved.record_path)?;
                            validate_record_id(
                                &record,
                                &resolved.expected_id,
                                &resolved.record_path,
                            )?;
                            Ok(record)
                        });
                let record = match record {
                    Ok(record) => record,
                    Err(err) => {
                        visit(Err(err))?;
                        continue;
                    }
                };
                let (status, observed_last_terminal_activity_at) = session_list_runtime_snapshot(
                    context,
                    &tmux_bin,
                    tmux_snapshots.as_ref(),
                    &record,
                );
                let observed_status = status.clone();
                let (record, status) = reconcile_startup_projection(
                    context,
                    record,
                    status,
                    tmux_snapshots
                        .as_ref()
                        .map(|snapshots| &snapshots.started_at),
                );
                let last_terminal_activity_at = if status == observed_status {
                    observed_last_terminal_activity_at
                } else {
                    last_terminal_activity_at(&tmux_bin, &record, &status)
                };
                let record = if status == "running" {
                    record
                } else {
                    backfill_provider_resume(context, record)
                };
                visit(Ok(session_view_from_parts(
                    context,
                    &record,
                    status,
                    last_terminal_activity_at,
                    &tmux_bin,
                    schedule_shadow_sampling,
                    coordination_registry
                        .as_ref()
                        .map(|registry| {
                            coordination::public_summary_from_registry(
                                context, registry, &record.id,
                            )
                        })
                        .unwrap_or_default(),
                    match query.as_mut() {
                        Some(query) => query.projection(context, &record),
                        None => context_usage::projection(context, &record),
                    },
                )))?;
            }
        }
    }
    Ok(())
}
