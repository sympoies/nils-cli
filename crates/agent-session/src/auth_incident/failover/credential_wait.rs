//! Bounded admission retry for a Claude replica that is not ready yet.
use serde::{Deserialize, Serialize};

const WINDOW_SECONDS: i64 = 120;
const FIRST_DELAY_SECONDS: i64 = 15;
const MAX_DELAY_SECONDS: i64 = 60;

pub(super) const STATE: &str = "waiting_credential";
pub(super) const TIMEOUT: &str = "credential_wait_deadline";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Wait {
    deadline_epoch: i64,
    next_attempt_epoch: i64,
    delay_seconds: i64,
}

impl Wait {
    pub(super) fn new(now: i64) -> Self {
        Self {
            deadline_epoch: now.saturating_add(WINDOW_SECONDS),
            next_attempt_epoch: now,
            delay_seconds: FIRST_DELAY_SECONDS,
        }
    }

    pub(super) fn expired(&self, now: i64) -> bool {
        now >= self.deadline_epoch
    }

    pub(super) fn due(&self, now: i64) -> bool {
        now >= self.next_attempt_epoch
    }

    pub(super) fn back_off(&mut self, now: i64) {
        self.next_attempt_epoch = now
            .saturating_add(self.delay_seconds)
            .min(self.deadline_epoch);
        self.delay_seconds = self.delay_seconds.saturating_mul(2).min(MAX_DELAY_SECONDS);
    }
}
