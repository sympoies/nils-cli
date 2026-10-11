//! Fail-closed observations for guarded terminal sends. These are not a native CAS.
use std::path::Path;
use std::time::{Duration, Instant};

use clap::Args;
use serde::Serialize;
use serde_json::json;

use crate::{
    CliContext, CliError, PANE_OBSERVATION_COMMAND_TIMEOUT, PANE_OBSERVATION_MAX_OUTPUT_BYTES,
    SessionRecord, activity, load_session_record, run_output_with_timeout_and_cap, send_submit,
    tmux_environment_value,
};

mod key;

#[derive(Debug, Default, Args)]
pub struct GuardArgs {
    /// Require the active runtime launch id to match before delivery.
    #[arg(long, value_name = "INCARNATION")]
    pub expect_runtime: Option<String>,
    /// Require the persisted provider session id to match before delivery.
    #[arg(long, value_name = "SESSION")]
    pub expect_provider_session: Option<String>,
    /// Require authoritative idle activity and no pending provider event.
    #[arg(long)]
    pub require_idle: bool,
    /// Refuse unless the provider composer is observably empty.
    #[arg(long)]
    pub require_empty_composer: bool,
    /// Persist an at-most-once delivery claim, retained on uncertain outcomes.
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,
}

impl GuardArgs {
    pub(crate) fn enabled(&self) -> bool {
        self.expect_runtime.is_some()
            || self.expect_provider_session.is_some()
            || self.require_idle
            || self.require_empty_composer
            || self.idempotency_key.is_some()
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct GuardReport {
    pub(crate) atomic: bool,
}

pub(crate) struct Admission<'a> {
    pub(crate) context: &'a CliContext,
    pub(crate) record: &'a SessionRecord,
    pub(crate) args: &'a GuardArgs,
    pub(crate) tmux_bin: &'a Path,
    pub(crate) verify_composer: bool,
    pub(crate) deadline: Instant,
}

impl Admission<'_> {
    pub(crate) fn check_key(&self) -> Result<(), CliError> {
        key::check(
            self.context,
            self.record,
            self.args.idempotency_key.as_deref(),
        )
    }

    pub(crate) fn claim_key(&self) -> Result<(), CliError> {
        key::claim(
            self.context,
            self.record,
            self.args.idempotency_key.as_deref(),
        )
    }

    pub(crate) fn check(&self, pasted_text: Option<&str>, stage: &str) -> Result<(), CliError> {
        self.timeout(stage)?;
        self.identity(stage)?;
        if (self.args.require_idle
            || self.args.require_empty_composer
            || self.verify_composer
            || pasted_text.is_some())
            && (!matches!(self.record.agent.as_str(), "claude" | "codex")
                || self
                    .record
                    .runtime
                    .as_ref()
                    .is_none_or(|runtime| runtime.kind != "tmux"))
        {
            return Err(self.refusal(
                "send-guard-unsupported",
                "provider does not support the requested terminal observation",
                "provider",
                stage,
            ));
        }
        self.idle(stage)?;
        if self.args.require_empty_composer || self.verify_composer || pasted_text.is_some() {
            let target = format!("{}:0.0", self.record.tmux_session);
            let input = capture_styled(self.tmux_bin, &target, self.timeout(stage)?)
                .and_then(|pane| send_submit::exact_composer_input(&self.record.agent, &pane))
                .ok_or_else(|| {
                    self.refusal(
                        "send-composer-unobservable",
                        "composer content cannot be observed exactly",
                        "empty_composer",
                        stage,
                    )
                })?;
            if let Some(expected) = pasted_text {
                if input != expected {
                    return Err(self.refusal(
                        "send-composer-mismatch",
                        "composer no longer contains exactly the submitted text",
                        "composer",
                        stage,
                    ));
                }
            } else if self.args.require_empty_composer && !input.is_empty() {
                return Err(self.refusal(
                    "send-composer-not-empty",
                    "composer contains a draft",
                    "empty_composer",
                    stage,
                ));
            }
        }
        // An ingest can queue during the pane read; probe again after it.
        self.idle(stage)?;
        self.timeout(stage)?;
        Ok(())
    }

    fn identity(&self, stage: &str) -> Result<(), CliError> {
        let current = load_session_record(self.context, &self.record.id)?;
        if crate::ensure_same_session_identity(self.record, &current).is_err()
            || current
                .provider_resume
                .as_ref()
                .map(|resume| (&resume.provider, &resume.session_id))
                != self
                    .record
                    .provider_resume
                    .as_ref()
                    .map(|resume| (&resume.provider, &resume.session_id))
        {
            return Err(self.refusal(
                "send-identity-mismatch",
                "session identity changed",
                "identity",
                stage,
            ));
        }
        let runtime = current
            .runtime
            .as_ref()
            .filter(|runtime| !runtime.launch_id.is_empty())
            .ok_or_else(|| {
                self.refusal(
                    "send-identity-unobservable",
                    "runtime identity is unavailable",
                    "identity",
                    stage,
                )
            })?;
        if self
            .args
            .expect_runtime
            .as_deref()
            .is_some_and(|expected| expected != runtime.launch_id)
            || self
                .args
                .expect_provider_session
                .as_deref()
                .is_some_and(|expected| {
                    current.provider_resume.as_ref().is_none_or(|resume| {
                        resume.provider != current.agent || resume.session_id != expected
                    })
                })
        {
            return Err(self.refusal(
                "send-identity-mismatch",
                "expected runtime or provider session identity does not match",
                "identity",
                stage,
            ));
        }
        let expected_state = crate::display_path(&self.context.state_dir);
        for (name, expected) in [
            ("AGENT_SESSION_ID", current.id.as_str()),
            ("AGENT_SESSION_STATE_DIR", expected_state.as_str()),
            ("AGENT_SESSION_RUNTIME_ID", runtime.launch_id.as_str()),
        ] {
            let actual = tmux_environment_value(
                self.tmux_bin,
                &current.tmux_session,
                name,
                self.timeout(stage)?,
            )
            .map_err(|_| {
                self.refusal(
                    "send-identity-unobservable",
                    "pane runtime identity cannot be observed",
                    "identity",
                    stage,
                )
            })?;
            if actual != expected {
                return Err(self.refusal(
                    "send-identity-mismatch",
                    "pane runtime identity does not match the session",
                    "identity",
                    stage,
                ));
            }
        }
        Ok(())
    }

    fn idle(&self, stage: &str) -> Result<(), CliError> {
        if !self.args.require_idle {
            return Ok(());
        }
        if activity::pending::event_pending(self.context, &self.record.id)? {
            return Err(self.refusal(
                "send-provider-event-pending",
                "provider event ingestion is pending",
                "idle",
                stage,
            ));
        }
        let state = activity::state_for_view(self.context, self.record)
            .filter(|state| {
                state.phase != activity::TurnPhase::Unknown
                    && state.source.confidence == activity::Confidence::Authoritative
            })
            .ok_or_else(|| {
                self.refusal(
                    "send-idle-unobservable",
                    "authoritative idle activity is unavailable",
                    "idle",
                    stage,
                )
            })?;
        if state.phase != activity::TurnPhase::Waiting || state.current_turn.is_some() {
            return Err(self.refusal(
                "send-session-not-idle",
                "session is not idle",
                "idle",
                stage,
            ));
        }
        Ok(())
    }

    fn timeout(&self, stage: &str) -> Result<Duration, CliError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(self.refusal(
                "send-guard-observation-timeout",
                "guard observation budget expired",
                "observation",
                stage,
            ));
        }
        Ok(remaining.min(PANE_OBSERVATION_COMMAND_TIMEOUT))
    }

    fn refusal(&self, code: &str, message: &str, guard: &str, stage: &str) -> CliError {
        CliError::runtime(
            code,
            message,
            Some(json!({"id": self.record.id,
            "guard": guard, "stage": stage, "atomic": false, "pasted": stage == "submit" && self.verify_composer, "submitted": false})),
        )
    }
}

fn capture_styled(tmux_bin: &Path, target: &str, timeout: Duration) -> Option<String> {
    let mut command = std::process::Command::new(tmux_bin);
    command.args(["capture-pane", "-p", "-e", "-t", target]);
    let output =
        run_output_with_timeout_and_cap(command, timeout, PANE_OBSERVATION_MAX_OUTPUT_BYTES)
            .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
