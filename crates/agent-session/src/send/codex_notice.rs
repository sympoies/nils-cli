//! One original-budget Codex notice attempt using the shared attestation owner.
use super::*;
use crate::provider_prompt::codex_notice::{self, LiveRollout};
use crate::provider_prompt::native_notice::{Baseline, Stamp};
use claude_notice::store;

pub(crate) mod first_wake;

enum Origin {
    Existing {
        baseline: Baseline,
        verify_at_write: bool,
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
                verify_at_write: false,
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
        first_wake_tmux: Option<&Path>,
        claim_key: impl FnOnce() -> Result<(), CliError>,
    ) -> Result<Self, CliError> {
        let origin = match (codex_notice::capture(record), first_wake_tmux) {
            (Some(baseline), _) => Origin::Existing {
                baseline,
                verify_at_write: false,
            },
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

    pub(crate) fn is_first_wake(&self) -> bool {
        matches!(self.origin, Origin::First(_))
    }

    pub(crate) fn verify_at_write(&self) -> bool {
        matches!(
            self.origin,
            Origin::First(_)
                | Origin::Existing {
                    verify_at_write: true,
                    ..
                }
        )
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
        let _ = store::finish(
            context,
            &self.token,
            None,
            store::Reason::HistoryUnavailable,
            self.started.elapsed(),
            self.deadline,
        );
        unconfirmed()
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
