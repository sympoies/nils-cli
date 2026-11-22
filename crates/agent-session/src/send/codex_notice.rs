//! One original-budget Codex notice attempt using the shared attestation owner.
use super::*;
use crate::provider_prompt::codex_notice::{self, LiveRollout};
use crate::provider_prompt::native_notice::{Baseline, Stamp};
use claude_notice::store;

pub(crate) mod first_wake;

enum Origin {
    Existing {
        baseline: Baseline,
        live_rollout: bool,
    },
    First(LiveRollout),
}

pub(crate) struct Prepared {
    origin: Origin,
    token: store::Token,
    started: Instant,
    deadline: Instant,
}

pub(crate) fn unconfirmed() -> CliError {
    CliError::runtime(
        "notice-native-unconfirmed",
        "native notice input is unconfirmed; do not replay it",
        None,
    )
}

impl Prepared {
    pub(crate) fn new(
        context: &CliContext,
        record: &SessionRecord,
        text: &str,
        started: Instant,
        deadline: Instant,
        recovery: Option<crate::auto_resume::notice::Recovery>,
        claim_key: impl FnOnce() -> Result<(), CliError>,
    ) -> Result<Self, CliError> {
        let baseline = codex_notice::capture(record)
            .ok_or_else(|| claude_notice::runtime::baseline_unavailable(record))?;
        Self::prepare(
            context,
            record,
            text,
            started,
            deadline,
            Origin::Existing {
                baseline,
                live_rollout: false,
            },
            recovery,
            claim_key,
        )
    }

    pub(crate) fn for_mailbox(
        context: &CliContext,
        record: &SessionRecord,
        text: &str,
        started: Instant,
        deadline: Instant,
        idle_tmux: Option<&Path>,
        claim_key: impl FnOnce() -> Result<(), CliError>,
    ) -> Result<Self, CliError> {
        let origin = match (codex_notice::capture(record), idle_tmux) {
            (Some(baseline), tmux) => {
                if let Some(tmux) = tmux {
                    first_wake::check(
                        context,
                        record,
                        tmux,
                        deadline,
                        first_wake::Check::Composer,
                    )?;
                }
                Origin::Existing {
                    baseline,
                    live_rollout: false,
                }
            }
            (None, Some(tmux)) => first_wake::observe(context, record, tmux, deadline)?,
            (None, None) => return Err(claude_notice::runtime::baseline_unavailable(record)),
        };
        Self::prepare(
            context, record, text, started, deadline, origin, None, claim_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare(
        context: &CliContext,
        record: &SessionRecord,
        text: &str,
        started: Instant,
        deadline: Instant,
        origin: Origin,
        recovery: Option<crate::auto_resume::notice::Recovery>,
        claim_key: impl FnOnce() -> Result<(), CliError>,
    ) -> Result<Self, CliError> {
        let stamp = match &origin {
            Origin::Existing { baseline, .. } => baseline.stamp.clone(),
            // No file existed at the provider-verified origin. Confirmation
            // binds the first provider-created file and scans from byte zero.
            Origin::First(_) => Stamp {
                device: 0,
                inode: 0,
                eof: 0,
                continuity_sha256: claude_notice::sha256(&[]),
            },
        };
        let claim = store::Claim::for_codex(
            record,
            text,
            stamp,
            deadline.saturating_duration_since(started),
            recovery,
        )?;
        let token = store::consume(context, claim, deadline, claim_key)?;
        Ok(Self {
            origin,
            token,
            started,
            deadline,
        })
    }

    pub(crate) fn idle_write_check(&self) -> first_wake::Check {
        match self.origin {
            Origin::First(_) => first_wake::Check::NeverUsed,
            Origin::Existing {
                live_rollout: true, ..
            } => first_wake::Check::LiveIdle,
            Origin::Existing {
                live_rollout: false,
                ..
            } => first_wake::Check::Composer,
        }
    }

    pub(crate) fn client_id(&self) -> &str {
        &self
            .token
            .claim
            .codex
            .as_ref()
            .expect("Codex claim")
            .client_message_id
    }

    pub(crate) fn unknown(self, context: &CliContext) -> CliError {
        self.finish_without_proof(context, store::Reason::HistoryUnavailable);
        unconfirmed()
    }

    pub(crate) fn drop_without_write(self, context: &CliContext) {
        self.finish_without_proof(context, store::Reason::Drop);
    }

    fn finish_without_proof(self, context: &CliContext, reason: store::Reason) {
        let _ = store::finish(
            context,
            &self.token,
            None,
            reason,
            self.started.elapsed(),
            self.deadline,
        );
    }

    /// The caller transfers/releases its record lock before provider ingress.
    pub(crate) fn confirm(
        mut self,
        context: &CliContext,
        text: &str,
        turn: &str,
    ) -> Result<(), CliError> {
        let id = self.client_id().to_owned();
        let proof = match &mut self.origin {
            Origin::Existing { baseline, .. } => codex_notice::confirm(
                baseline,
                &id,
                text,
                turn,
                &self.token.claim.binding.provider_session_id,
                self.deadline,
            ),
            Origin::First(first) => first.confirm(&id, text, turn, self.deadline),
        };
        let locked = crate::acquire_session_record_lock_timed(
            context,
            &self.token.claim.binding.launch.session_id,
            self.deadline.saturating_duration_since(Instant::now()),
        );
        let unchanged = locked.is_ok()
            && load_session_record(context, &self.token.claim.binding.launch.session_id)
                .ok()
                .and_then(|record| crate::serve::prompt_receipts::native_identity(&record))
                .as_ref()
                == Some(
                    &self
                        .token
                        .claim
                        .codex
                        .as_ref()
                        .expect("Codex claim")
                        .identity_sha256,
                );
        let proof = proof.filter(|_| unchanged);
        store::finish(
            context,
            &self.token,
            proof.as_ref(),
            store::Reason::IdentityChanged,
            self.started.elapsed(),
            self.deadline,
        )
        .map_err(|_| unconfirmed())?;
        proof.map(|_| ()).ok_or_else(unconfirmed)
    }
}
