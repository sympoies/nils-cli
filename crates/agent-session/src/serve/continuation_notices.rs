//! Scheduler producers share canonical notice preparation.
use super::*;

pub(super) async fn process_codex_capacity_auto_resume_id(
    state: Arc<ServeState>,
    target: CodexAutoResumeTarget,
) {
    let control = state
        .codex_controls
        .lock()
        .ok()
        .and_then(|controls| controls.get(&target.id).cloned());
    let Some(control) = control else {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "control_unavailable",
        )
        .await;
        return;
    };
    if control.launch_id != target.launch_id {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "control_unavailable",
        )
        .await;
        return;
    }
    let state_tmux = state.tmux_bin.clone();
    let context = state.context.clone();
    let runtime = tokio::runtime::Handle::current();
    let handle = control.handle.clone();
    let expected_launch_id = target.launch_id;
    let id = target.id;
    tokio::task::spawn_blocking(move || {
        let now_epoch = jiff::Timestamp::now().as_second();
        if let Err(err) = auto_resume::tick_for_runtime_and_binding(
            &context,
            &id,
            &expected_launch_id,
            auto_resume::RuntimeBindingTick {
                binding: &target.binding,
                failover_account: None,
            },
            now_epoch,
            &UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |record, record_lock| {
                crate::send::daemon_notice::send_codex(
                    &context,
                    record,
                    record_lock,
                    auto_resume::CAPACITY_CONTINUATION_MESSAGE,
                    &state_tmux,
                    &runtime,
                    &handle,
                )
            },
        ) {
            eprintln!(
                "warning: Codex capacity auto-resume tick failed: {}",
                err.code()
            );
        }
    })
    .await
    .ok();
}

pub(super) async fn process_codex_auto_resume_id(
    state: Arc<ServeState>,
    target: CodexAutoResumeTarget,
) {
    let control = state
        .codex_controls
        .lock()
        .ok()
        .and_then(|controls| controls.get(&target.id).cloned());
    let Some(control) = control else {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "usage_unavailable",
        )
        .await;
        return;
    };
    if control.launch_id != target.launch_id {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "usage_unavailable",
        )
        .await;
        return;
    }
    let usage = match control.handle.usage().await {
        Ok(usage) => usage,
        Err(_) => {
            record_codex_scheduler_error_for_runtime(
                state.context.clone(),
                target.id,
                target.launch_id,
                "usage_unavailable",
            )
            .await;
            return;
        }
    };
    let state_tmux = state.tmux_bin.clone();
    let context = state.context.clone();
    let runtime = tokio::runtime::Handle::current();
    let handle = control.handle.clone();
    let expected_launch_id = target.launch_id;
    let id = target.id;
    tokio::task::spawn_blocking(move || {
        let now_epoch = jiff::Timestamp::now().as_second();
        let failover_account = auto_resume::failover_selection_request(
            &context,
            &id,
            &expected_launch_id,
            &target.binding,
            usage.has_exhausted_windows,
        )
        .ok()
        .flatten()
        .and_then(|request| {
            crate::codex_account::select_next_account(&request.after, &request.excluded)
                .ok()
                .flatten()
        })
        .map(|account| account.account);
        if let Err(err) = auto_resume::tick_for_runtime_and_binding(
            &context,
            &id,
            &expected_launch_id,
            auto_resume::RuntimeBindingTick {
                binding: &target.binding,
                failover_account: failover_account.as_deref(),
            },
            now_epoch,
            &usage,
            |record, record_lock| {
                crate::send::daemon_notice::send_codex(
                    &context,
                    record,
                    record_lock,
                    auto_resume::CONTINUATION_MESSAGE,
                    &state_tmux,
                    &runtime,
                    &handle,
                )
            },
        ) {
            eprintln!("warning: Codex auto-resume tick failed: {}", err.code());
        }
    })
    .await
    .ok();
}
