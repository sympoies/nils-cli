//! Inspect the exact idle Codex rollout when history cannot supply a baseline.
use super::*;
use crate::codex_app_server::guarded::Client;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) enum Check {
    Composer,
    LiveIdle,
    NeverUsed,
}

pub(super) fn observe(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
) -> Result<Origin, CliError> {
    let missing = || claude_notice::runtime::baseline_unavailable(record);
    check(context, record, tmux, deadline, Check::LiveIdle)?;
    let typed_error = |error: crate::codex_app_server::guarded::Error| {
        error.annotate(CliError::unavailable(
            error.code(),
            "Codex wake idle proof failed",
            None,
        ))
    };
    let mut client = Client::connect(record, deadline).map_err(typed_error)?;
    let snapshot = client.idle_thread(false).map_err(typed_error)?;
    let path = snapshot
        .pointer("/thread/path")
        .and_then(Value::as_str)
        .filter(|_| {
            snapshot
                .pointer("/thread/status/type")
                .and_then(Value::as_str)
                == Some("idle")
        })
        .map(Path::new)
        .ok_or_else(missing)?;
    let rollout = LiveRollout::observe(record, path).ok_or_else(missing)?;
    if let Some(baseline) = rollout.pin() {
        // The history scan may miss a readable, exact live rollout.
        return Ok(Origin::Existing {
            baseline,
            live_rollout: true,
        });
    }
    if rollout.is_absent() && never_used(context, record) {
        return Ok(Origin::First(rollout));
    }
    Err(missing())
}

pub(crate) fn check(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
    check: Check,
) -> Result<(), CliError> {
    if matches!(check, Check::NeverUsed) && !never_used(context, record) {
        return Err(claude_notice::runtime::baseline_unavailable(record));
    }
    let args = send_guard::GuardArgs {
        require_idle: !matches!(check, Check::Composer),
        require_empty_composer: true,
        expect_runtime: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone()),
        expect_provider_session: record
            .provider_resume
            .as_ref()
            .map(|provider| provider.session_id.clone()),
        ..Default::default()
    };
    send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &args,
        tmux_bin: tmux,
        verify_composer: true,
        deadline,
    }
    .check_preserving_draft("admission")
}

fn never_used(context: &CliContext, record: &SessionRecord) -> bool {
    crate::activity::state_for_view(context, record).is_some_and(|turn| {
        turn.phase == crate::activity::TurnPhase::Waiting
            && turn.source.confidence == crate::activity::Confidence::Authoritative
            && turn.current_turn.is_none()
            && turn.last_turn.is_none()
    })
}
