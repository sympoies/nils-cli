//! Wake attempt admission for a locked recipient quiescence guard.
use super::*;

/// Error code of a notification refused because the recipient is not quiescent;
/// its details carry the blocking `condition`.
pub(crate) const NOTIFICATION_NOT_QUIESCENT_CODE: &str = "coordination-notification-not-quiescent";

impl SessionQuiescenceGuard {
    /// The first recipient condition that forbids a wake attempt. With
    /// `claim_blocks` false the recipient's own claim, which says what it
    /// works on rather than that it is busy, is not a condition; only an
    /// operation under it is.
    pub(crate) fn notification_block(&self, claim_blocks: bool) -> Option<&'static str> {
        if auth_effects::quarantined(&self._locked.path, &self.session_id, &self.incarnation) {
            Some("auth-quarantined")
        } else if !self.broker_authoritative {
            Some("broker-not-authoritative")
        } else if claim_blocks && self.active_claim {
            Some("active-claim")
        } else if self.active_operation {
            Some("active-operation")
        } else if self.uncertain_operation {
            Some("uncertain-operation")
        } else if crate::restart::fenced(&self._locked.path, &self.session_id, &self.incarnation) {
            Some("restart-fenced")
        } else {
            None
        }
    }

    /// A running turn is in flight, so any claim of the recipient also defers
    /// the in-turn checkpoint.
    pub(crate) fn begin_notification_attempt(
        &mut self,
        candidate: &NotificationCandidate,
    ) -> Result<bool, CliError> {
        self.begin_attempt(candidate, true)
    }

    /// An idle recipient is woken while it holds only its own claim.
    pub(crate) fn begin_idle_notification_attempt(
        &mut self,
        candidate: &NotificationCandidate,
    ) -> Result<bool, CliError> {
        self.begin_attempt(candidate, false)
    }

    fn begin_attempt(
        &mut self,
        candidate: &NotificationCandidate,
        claim_blocks: bool,
    ) -> Result<bool, CliError> {
        if candidate.target_session_id != self.session_id
            || candidate.target_incarnation != self.incarnation
            || self.notification_block(claim_blocks).is_some()
        {
            return Ok(false);
        }
        let attempted_at = jiff::Timestamp::now();
        // The recipient may have drained its inbox on its own since the
        // generation was queued; a reminder then only prompts a stale check.
        if !notification::has_live_unread(
            &self._locked.registry,
            &self.session_id,
            &self.incarnation,
            attempted_at.as_second(),
        ) {
            if notification::resolve_drained_generation(
                &mut self._locked.registry,
                &self.session_id,
                &self.incarnation,
                attempted_at.as_second(),
            ) {
                self._locked.save()?;
            }
            return Ok(false);
        }
        let Some(_) = notification::transition_attempt_at(
            &mut self._locked.registry,
            candidate,
            attempted_at.as_second(),
            attempted_at,
        ) else {
            return Ok(false);
        };
        self._locked.save()?;
        Ok(true)
    }
}
