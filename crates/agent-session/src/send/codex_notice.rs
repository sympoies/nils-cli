//! One original-budget Codex notice attempt using the shared attestation owner.
use super::*;
use claude_notice::store;

pub(crate) struct Prepared {
    baseline: crate::provider_prompt::native_notice::Baseline,
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
        let baseline = crate::provider_prompt::codex_notice::capture(record).ok_or_else(|| {
            CliError::runtime(
                "notice-native-baseline-unavailable",
                "native notice requires a bound provider transcript baseline",
                None,
            )
        })?;
        let claim = store::Claim::for_codex(
            record,
            text,
            baseline.stamp.clone(),
            deadline.saturating_duration_since(started),
            recovery,
        )?;
        let token = store::consume(context, claim, deadline, claim_key)?;
        Ok(Self {
            baseline,
            token,
            started,
            deadline,
        })
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
        let proof = crate::provider_prompt::codex_notice::confirm(
            &mut self.baseline,
            &id,
            text,
            turn,
            &self.token.claim.binding.provider_session_id,
            self.deadline,
        );
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
