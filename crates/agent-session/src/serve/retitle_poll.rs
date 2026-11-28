//! Automatic retitling only needs a live tmux inventory when an eligible
//! durable turn can actually start or retry an operation.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;

use super::{RetitleV3ContinuationGuard, ServeState, execute_retitle_v3, retitle_worker_failed};
use crate::{CliContext, CliError, activity::TurnState, display_metadata::TitleMode};

#[derive(Clone)]
pub(super) struct AutomaticRetitleCandidate {
    id: String,
    pub(super) tracking_key: String,
    pub(super) request: crate::retitle_v3::RetitleV3Request,
}

#[derive(Default)]
pub(super) struct AutomaticRetitleTracker {
    entries: HashMap<String, AutomaticRetitleAttempt>,
}

struct AutomaticRetitleAttempt {
    attempts: u8,
    retry_after: Instant,
    in_flight: bool,
    terminal: bool,
}

impl AutomaticRetitleAttempt {
    fn ready(&self, now: Instant) -> bool {
        !self.terminal && !self.in_flight && now >= self.retry_after
    }
}

fn automatic_retitle_scanner_error_is_retryable(code: &str) -> bool {
    code == "retitle-v3-history-unavailable"
        || crate::retitle::is_retryable_automatic_error_code(code)
}

impl AutomaticRetitleTracker {
    fn ready(&self, key: &str, now: Instant) -> bool {
        self.entries
            .get(key)
            .is_none_or(|attempt| attempt.ready(now))
    }

    pub(super) fn begin(&mut self, key: &str, now: Instant) -> bool {
        match self.entries.entry(key.to_string()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(AutomaticRetitleAttempt {
                    attempts: 1,
                    retry_after: now,
                    in_flight: true,
                    terminal: false,
                });
                true
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let attempt = entry.get_mut();
                if !attempt.ready(now) {
                    return false;
                }
                attempt.attempts = attempt.attempts.saturating_add(1);
                attempt.in_flight = true;
                true
            }
        }
    }

    pub(super) fn finish(&mut self, key: &str, result: Result<&str, &CliError>, now: Instant) {
        let Some(attempt) = self.entries.get_mut(key) else {
            return;
        };
        attempt.in_flight = false;
        match result {
            Ok("terminal") => attempt.terminal = true,
            Ok(_) => {
                // An accepted durable operation is still pending. Polling it
                // must not consume the bounded provider-failure retry budget.
                attempt.attempts = attempt.attempts.saturating_sub(1);
                attempt.retry_after = now + Duration::from_secs(1);
            }
            Err(error)
                if automatic_retitle_scanner_error_is_retryable(error.code())
                    && attempt.attempts < crate::retitle::MAX_AUTOMATIC_ATTEMPTS =>
            {
                let shift = u32::from(attempt.attempts.saturating_sub(1));
                attempt.retry_after = now + Duration::from_secs(1_u64 << shift);
            }
            Err(_) => attempt.terminal = true,
        }
    }

    fn prune(&mut self) {
        if self.entries.len() <= 4096 {
            return;
        }
        self.entries
            .retain(|_, attempt| !attempt.terminal || attempt.in_flight);
        if self.entries.len() > 4096 {
            self.entries.clear();
        }
    }
}

pub(super) fn automatic_retitle_candidate(
    session: &crate::SessionView,
) -> Option<AutomaticRetitleCandidate> {
    if session.title_mode == TitleMode::Pinned
        || session.status != "running"
        || session.provider_resume.is_none()
    {
        return None;
    }
    RetitleInput {
        id: &session.id,
        incarnation: session.session_incarnation.as_deref()?,
        title_revision: session.title_revision,
        has_title: session.title.is_some(),
        memory_revision: session.retitle_v3_memory_revision,
        state: session.turn_state.as_ref()?,
    }
    .candidate()
}

struct RetitleInput<'a> {
    id: &'a str,
    incarnation: &'a str,
    title_revision: u64,
    has_title: bool,
    memory_revision: u64,
    state: &'a TurnState,
}

impl RetitleInput<'_> {
    fn candidate(self) -> Option<AutomaticRetitleCandidate> {
        let Self {
            id,
            incarnation,
            title_revision,
            has_title,
            memory_revision,
            state,
        } = self;
        let (turn_id, trigger) = if let Some(turn_id) = state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_deref())
        {
            let trigger = if title_revision == 0 && !has_title {
                crate::retitle::RetitleTrigger::Initial
            } else {
                crate::retitle::RetitleTrigger::Prompt
            };
            (turn_id, trigger)
        } else {
            (
                state
                    .last_turn
                    .as_ref()
                    .and_then(|turn| turn.provider_turn_id.as_deref())?,
                crate::retitle::RetitleTrigger::CompletionRecovery,
            )
        };
        let tracking_identity = format!("{incarnation}\0{turn_id}\0{trigger:?}");
        let request_identity = format!(
            "{tracking_identity}\0{}\0{}",
            state.revision, title_revision
        );
        // The request key identifies the latest observation, while v3's automatic
        // operation fingerprint intentionally coalesces all revisions of this
        // incarnation/provider-turn pair into one durable operation.
        Some(AutomaticRetitleCandidate {
            id: id.to_owned(),
            tracking_key: format!(
                "auto-track-{}",
                crate::retitle::hash_identity(&tracking_identity).trim_start_matches("sha256:")
            ),
            request: crate::retitle_v3::RetitleV3Request {
                schema_version: crate::retitle_v3::REQUEST_SCHEMA.to_string(),
                trigger: "automatic".to_string(),
                idempotency_key: format!(
                    "auto-{}",
                    crate::retitle::hash_identity(&request_identity).trim_start_matches("sha256:")
                ),
                expected: crate::retitle_v3::RetitleV3FenceInput {
                    session_incarnation: incarnation.to_string(),
                    title_revision,
                    memory_revision,
                    activity_revision: Some(state.revision),
                    provider_turn_id: Some(turn_id.to_string()),
                },
            },
        })
    }
}

fn discover_candidates(context: &CliContext) -> Vec<AutomaticRetitleCandidate> {
    super::tmux_poll::records(context)
        .filter_map(|record| {
            if record.provider_resume.is_none()
                || crate::display_metadata::title_mode(&record) == TitleMode::Pinned
            {
                return None;
            }
            let runtime = record
                .runtime
                .as_ref()
                .filter(|runtime| !runtime.launch_id.is_empty())?;
            let state = crate::activity::state_for_view(context, &record)?;
            RetitleInput {
                id: &record.id,
                incarnation: &runtime.launch_id,
                title_revision: record.title_revision,
                has_title: record.title.is_some(),
                memory_revision: crate::retitle_v3::memory_revision(&record),
                state: &state,
            }
            .candidate()
        })
        .collect()
}

pub(super) async fn auto_retitle_loop(state: Arc<ServeState>) {
    let mut tracker = AutomaticRetitleTracker::default();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if state.retitle.configured_provider_kind().is_none() {
            continue;
        }
        let context = state.context.clone();
        let Ok(mut candidates) =
            tokio::task::spawn_blocking(move || discover_candidates(&context)).await
        else {
            continue;
        };
        let now = Instant::now();
        candidates.retain(|candidate| {
            tracker.ready(&candidate.tracking_key, now)
                && !RetitleV3ContinuationGuard::is_registered(
                    &state.retitle_v3_continuations,
                    &crate::retitle_v3::request_operation_hash(&candidate.id, &candidate.request),
                )
        });
        if candidates.is_empty() {
            continue;
        }
        // The cheap scan is only a filter. Use the canonical live view
        // to admit operations and keep its startup-reconciliation semantics.
        let context = state.context.clone();
        let tmux = state.tmux_bin.clone();
        let collect = state.session_collector.clone();
        let Ok(Ok(sessions)) = tokio::task::spawn_blocking(move || collect(&context, &tmux)).await
        else {
            continue;
        };
        let mut tasks = JoinSet::new();
        let mut pending_keys = HashSet::new();
        for candidate in sessions.iter().filter_map(automatic_retitle_candidate) {
            let operation_hash =
                crate::retitle_v3::request_operation_hash(&candidate.id, &candidate.request);
            if RetitleV3ContinuationGuard::is_registered(
                &state.retitle_v3_continuations,
                &operation_hash,
            ) {
                continue;
            }
            if !tracker.begin(&candidate.tracking_key, now) {
                continue;
            }
            let key = candidate.tracking_key;
            pending_keys.insert(key.clone());
            let state = state.clone();
            tasks.spawn(async move {
                let result = execute_retitle_v3(state, candidate.id, candidate.request).await;
                (key, result)
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok((key, result)) = result {
                pending_keys.remove(&key);
                let outcome = match result.as_ref() {
                    Ok((_, response)) => Ok(response.status.as_str()),
                    Err(error) => Err(error),
                };
                tracker.finish(&key, outcome, Instant::now());
            }
        }
        let worker_failed = retitle_worker_failed();
        for key in pending_keys {
            tracker.finish(&key, Err(&worker_failed), Instant::now());
        }
        tracker.prune();
    }
}
