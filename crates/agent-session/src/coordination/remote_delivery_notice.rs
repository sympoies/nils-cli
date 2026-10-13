//! Daemon-authored notices use existing mailbox persistence and receipt deduplication.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{Address, Config, Envelope, Origin, Retained, digest_bytes, now_epoch};
use crate::{CliContext, CliError};

const BATCH_SIZE: usize = 8;
type SenderKey = (String, String);

#[derive(Default)]
pub(crate) struct State {
    scanned_len: usize,
    last_scanned_id: Option<String>,
    next_expiry: Option<i64>,
    senders: BTreeMap<SenderKey, Pending>,
    order: VecDeque<SenderKey>,
    #[cfg(test)]
    scanned: usize,
    #[cfg(test)]
    receive_attempts: usize,
}

struct Pending {
    items: VecDeque<Retained>,
    retry_at: i64,
}

fn expires_at(item: &Retained) -> Option<i64> {
    item.state_changed_at_epoch.map(|created| {
        item.expires_at_epoch
            .saturating_add(86400)
            .min(created.saturating_add(super::super::mailbox::MAX_EXPIRY_SECS))
    })
}

fn notice_id(item: &Retained) -> String {
    let digest = digest_bytes(format!("remote-delivery-notice:{}", item.message_id).as_bytes());
    uuid::Uuid::parse_str(&digest[..32])
        .expect("hexadecimal digest")
        .to_string()
}

fn envelope(config: &Config, item: &Retained, sender: &Address) -> Envelope<Origin> {
    let reason = item
        .reason
        .as_deref()
        .and_then(super::relay_reason_code)
        .unwrap_or("remote-delivery-failed");
    let recovery = if item.state == "delivery-unknown" {
        "Delivery is unconfirmed. Inspect the recipient before retrying; a new send may duplicate delivery."
    } else {
        "Review the reason before retrying; use a new idempotency key for a new send."
    };
    let body = format!(
        "Remote message {} ended in {} ({reason}). Inspect it with \
agent-session message delivery --session <sender-session> --message {} --format json. \
{recovery}",
        item.message_id, item.state, item.message_id
    );
    let created = item
        .state_changed_at_epoch
        .expect("selected terminal timestamp");
    Envelope {
        schema_version: super::SERVICE_ENVELOPE_V2.into(),
        message_id: notice_id(item),
        from: Origin::Service(super::super::service::Origin {
            machine: config.machine.clone(),
            service_id: "agent-session-delivery".into(),
            service_generation: "v1".into(),
        }),
        to: sender.clone(),
        body_sha256: digest_bytes(body.as_bytes()),
        body,
        created_at_epoch: created,
        expires_at_epoch: expires_at(item).expect("selected terminal timestamp"),
        reply_to: None,
        reply_depth: 0,
        category: Some(crate::cli::MessageCategory::Report),
        forwarding: None,
    }
}

/// Terminal source identities already survive restart without bodies. A fixed
/// notice ID and the existing ingress receipt preserve deduplication through
/// that identity's retention, including after inbox acknowledgement.
impl State {
    fn purge_expired(&mut self, now: i64) {
        if self.next_expiry.is_none_or(|expiry| expiry > now) {
            return;
        }
        self.next_expiry = None;
        self.senders.retain(|_, pending| {
            pending
                .items
                .retain(|item| expires_at(item).is_some_and(|expiry| expiry > now));
            for item in &pending.items {
                if let Some(expiry) = expires_at(item) {
                    self.next_expiry =
                        Some(self.next_expiry.map_or(expiry, |next| next.min(expiry)));
                }
            }
            !pending.items.is_empty()
        });
        self.order.retain(|key| self.senders.contains_key(key));
    }

    fn refresh(&mut self, context: &CliContext, config: &Config) -> Result<(), CliError> {
        let now = now_epoch();
        self.purge_expired(now);
        let (candidates, len, last_id) = {
            let journal = super::lock_journal(context)?;
            let retained = &journal.registry.retained;
            // Terminal identities append until pruning. Keep the unprocessed
            // tail across wakes; rebuild only when pruning changes its boundary.
            let same_prefix = self.scanned_len <= retained.len()
                && self.last_scanned_id.as_deref()
                    == self
                        .scanned_len
                        .checked_sub(1)
                        .and_then(|index| retained.get(index))
                        .map(|item| item.message_id.as_str());
            if !same_prefix {
                self.scanned_len = 0;
                self.senders.clear();
                self.order.clear();
                self.next_expiry = None;
            }
            #[cfg(test)]
            {
                self.scanned += retained.len() - self.scanned_len;
            }
            let candidates = retained[self.scanned_len..].iter()
                .filter(|item| matches!(item.state.as_str(), "rejected" | "delivery-unknown")
                    && expires_at(item).is_some_and(|expiry| expiry > now)
                    && matches!(&item.sender, Origin::Session(sender) if sender.machine == config.machine))
                .cloned().collect::<Vec<_>>();
            (
                candidates,
                retained.len(),
                retained.last().map(|item| item.message_id.clone()),
            )
        };
        // Do not advance the cursor until the receipt snapshot succeeds.
        if candidates.is_empty() {
            self.scanned_len = len;
            self.last_scanned_id = last_id;
            return Ok(());
        }
        let registry = super::super::lock_registry_observational(context)?;
        for item in candidates {
            if has_receipt(&registry.registry, &item, now) {
                continue;
            }
            let Origin::Session(sender) = &item.sender else {
                continue;
            };
            let key = (
                sender.session_id.clone(),
                sender.session_incarnation.clone(),
            );
            let pending = self.senders.entry(key.clone()).or_insert_with(|| {
                self.order.push_back(key);
                Pending {
                    items: VecDeque::new(),
                    retry_at: 0,
                }
            });
            if let Some(expiry) = expires_at(&item) {
                self.next_expiry = Some(self.next_expiry.map_or(expiry, |next| next.min(expiry)));
            }
            pending.items.push_back(item);
        }
        self.scanned_len = len;
        self.last_scanned_id = last_id;
        Ok(())
    }

    fn next(&mut self, now: i64, deferred: &BTreeSet<SenderKey>) -> Option<(SenderKey, Retained)> {
        for _ in 0..self.order.len() {
            let key = self.order.pop_front()?;
            let pending = self.senders.get_mut(&key).expect("scheduled sender");
            if pending.retry_at <= now
                && !deferred.contains(&key)
                && let Some(item) = pending.items.pop_front()
            {
                return Some((key, item));
            }
            self.order.push_back(key);
        }
        None
    }

    fn finish(&mut self, key: &SenderKey, retry: Option<Retained>) {
        let pending = self.senders.get_mut(key).expect("selected sender");
        if let Some(item) = retry {
            pending.items.push_front(item);
            // Admission work may take time. Back off from completion, never
            // from the pass start, and defer all siblings after one refusal.
            pending.retry_at = now_epoch().saturating_add(super::RETRY_SECS);
        } else if pending.items.is_empty() {
            self.senders.remove(key);
            return;
        }
        self.order.push_back(key.clone());
    }

    pub(crate) fn reconcile(
        &mut self,
        context: &CliContext,
        config: &Config,
    ) -> Result<Option<i64>, CliError> {
        self.refresh(context, config)?;
        let mut deferred = BTreeSet::new();
        for _ in 0..BATCH_SIZE {
            let Some((key, item)) = self.next(now_epoch(), &deferred) else {
                break;
            };
            match self.deliver(context, config, &item) {
                Ok(true) => self.finish(&key, None),
                Ok(false) => {
                    deferred.insert(key.clone());
                    self.finish(&key, Some(item));
                }
                Err(error) => {
                    self.finish(&key, Some(item));
                    return Err(error);
                }
            }
        }
        let now = now_epoch();
        self.purge_expired(now);
        for key in deferred {
            if let Some(pending) = self.senders.get_mut(&key) {
                pending.retry_at = now.saturating_add(super::RETRY_SECS);
            }
        }
        Ok(self
            .senders
            .values()
            .map(|pending| pending.retry_at.max(now))
            .min())
    }

    fn deliver(
        &mut self,
        context: &CliContext,
        config: &Config,
        item: &Retained,
    ) -> Result<bool, CliError> {
        let now = now_epoch();
        if expires_at(item).is_none_or(|expiry| expiry <= now) {
            return Ok(true);
        }
        let Origin::Session(sender) = &item.sender else {
            return Ok(true);
        };
        let ready = {
            let registry = super::super::lock_registry_observational(context)?;
            if has_receipt(&registry.registry, item, now) {
                return Ok(true);
            }
            let broker = registry.registry.brokers.get(&sender.session_id);
            if broker.is_some_and(|broker| broker.incarnation != sender.session_incarnation) {
                return Ok(true);
            }
            broker.is_some_and(|broker| broker.state == "ready")
        };
        let record = match crate::load_session_record(context, &sender.session_id) {
            Ok(record) => record,
            Err(error) if matches!(error.code(), "session-not-found" | "message-not-found") => {
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        if super::incarnation(&record)? != sender.session_incarnation {
            return Ok(true);
        }
        if !ready {
            return Ok(false);
        }
        // receive takes the session lock and rechecks incarnation and readiness.
        // No journal/registry guard spans this local mailbox transaction.
        #[cfg(test)]
        {
            self.receive_attempts += 1;
        }
        match super::receive(context, &config.machine, envelope(config, item, sender)) {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.code(),
                    "session-incarnation-conflict" | "session-not-found"
                ) =>
            {
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

fn has_receipt(registry: &super::super::Registry, item: &Retained, now: i64) -> bool {
    let key = super::super::receipt_key(
        super::RECEIVE_PRINCIPAL,
        super::RECEIVE_INCARNATION,
        super::RECEIVE_OPERATION,
        &notice_id(item),
    );
    registry
        .receipts
        .get(&key)
        .is_some_and(|receipt| receipt.expires_at_epoch > now)
}

#[cfg(test)]
pub(super) fn reconcile(context: &CliContext, config: &Config) -> Result<Option<i64>, CliError> {
    State::default().reconcile(context, config)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{config, envelope as session_envelope, fixture, own_outbox};
    use super::*;
    use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn retain_failure(context: &CliContext, key: &str, terminal: i64, expires: i64) {
        let mut item = own_outbox(key, "rejected");
        item.envelope.created_at_epoch = terminal;
        item.envelope.expires_at_epoch = expires;
        item.state_changed_at_epoch = Some(terminal);
        item.reason = Some("remote-messaging-unsupported".into());
        item.attempts = 1;
        let mut journal = super::super::lock_journal(context).unwrap();
        journal.registry.retained.push(item.identity());
        journal.save().unwrap();
    }

    fn notice_count(context: &CliContext) -> usize {
        super::super::lock_registry(context)
            .unwrap()
            .registry
            .messages
            .len()
    }

    #[test]
    fn successful_notice_backlog_drains_in_bounded_batches() {
        let (_temp, context) = fixture();
        let now = now_epoch();
        for index in 0..32 {
            retain_failure(&context, &format!("batch-{index}"), now, now + 60);
        }
        let mut delivered = 0;
        for _ in 0..8 {
            reconcile(&context, &config()).unwrap();
            let current = notice_count(&context);
            assert!(current - delivered <= 8, "one wake must bound deliveries");
            delivered = current;
            if delivered == 32 {
                break;
            }
        }
        assert_eq!(
            delivered, 32,
            "successful backlog must drain in batches instead of one full pass per notice"
        );
    }

    #[test]
    fn expired_unpersisted_notice_does_not_schedule_retry() {
        let (_temp, context) = fixture();
        let now = now_epoch();
        let terminal = now - super::super::super::mailbox::MAX_EXPIRY_SECS;
        retain_failure(&context, "expired-notice", terminal, now);
        assert_eq!(
            reconcile(&context, &config()).unwrap(),
            None,
            "notice envelope expired while its source identity is still retained"
        );
        assert_eq!(notice_count(&context), 0);
    }

    #[test]
    fn persisted_notice_receipt_suppresses_retry() {
        let (_temp, context) = fixture();
        let now = now_epoch();
        retain_failure(&context, "persisted-notice", now, now + 60);
        reconcile(&context, &config()).unwrap();
        assert_eq!(notice_count(&context), 1);
        assert_eq!(reconcile(&context, &config()).unwrap(), None);
        assert_eq!(notice_count(&context), 1);
    }

    #[test]
    fn successful_backlog_cursor_does_not_rescan_delivered_identities() {
        let (_temp, context) = fixture();
        let now = now_epoch();
        for index in 0..32 {
            retain_failure(&context, &format!("cursor-{index}"), now, now + 60);
        }
        let mut state = State::default();
        for wake in 1..=4 {
            state.reconcile(&context, &config()).unwrap();
            assert_eq!(notice_count(&context), wake * BATCH_SIZE);
            assert_eq!(state.scanned, 32, "draining must retain its source cursor");
            assert_eq!(state.receive_attempts, wake * BATCH_SIZE);
        }
        assert_eq!(state.reconcile(&context, &config()).unwrap(), None);
        assert_eq!(state.scanned, 32);
        assert_eq!(state.receive_attempts, 32);
    }

    #[test]
    fn cursor_discovers_appends_and_recovers_from_source_pruning() {
        let (_temp, context) = fixture();
        let now = now_epoch();
        for index in 0..16 {
            retain_failure(&context, &format!("pruning-{index}"), now, now + 60);
        }
        let mut state = State::default();
        state.reconcile(&context, &config()).unwrap();
        assert_eq!(notice_count(&context), BATCH_SIZE);
        retain_failure(&context, "appended", now, now + 60);
        state.reconcile(&context, &config()).unwrap();
        assert_eq!(notice_count(&context), 16);
        assert_eq!(
            state.scanned, 17,
            "append scans only the newly retained tail"
        );
        {
            let mut journal = super::super::lock_journal(&context).unwrap();
            journal.registry.retained.drain(..BATCH_SIZE);
            journal.save().unwrap();
        }
        // Rebuilding after pruning filters persisted receipts instead of
        // duplicating the notices that were delivered before the boundary moved.
        assert_eq!(state.reconcile(&context, &config()).unwrap(), None);
        assert_eq!(notice_count(&context), 17);
        assert_eq!(state.receive_attempts, 17);
    }

    #[test]
    fn supported_backlog_defers_quota_blocked_sender_and_delivers_other_sender() {
        let (_temp, context) = fixture();
        let second = "another-recipient";
        let second_incarnation = "another-incarnation";
        let directory = context
            .state_dir
            .join("sessions")
            .join(second)
            .join("coordination");
        fs::create_dir_all(&directory).unwrap();
        for path in [directory.parent().unwrap(), directory.as_path()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut record: Value = serde_json::from_slice(
            &fs::read(context.state_dir.join("sessions/recipient/session.json")).unwrap(),
        )
        .unwrap();
        record["id"] = json!(second);
        record["runtime"]["launch_id"] = json!(second_incarnation);
        write_atomic(
            &directory.parent().unwrap().join("session.json"),
            &serde_json::to_vec(&record).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let capability = fs::read(super::super::super::capability_path(
            &context,
            "recipient",
            "recipient-incarnation",
        ))
        .unwrap();
        write_atomic(
            &super::super::super::capability_path(&context, second, second_incarnation),
            &capability,
            SECRET_FILE_MODE,
        )
        .unwrap();
        write_atomic(
            &directory.join("heartbeat"),
            format!("{second_incarnation}:{}\n", now_epoch()).as_bytes(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        super::super::receive(&context, &config().machine, session_envelope()).unwrap();
        {
            let mut registry = super::super::super::lock_registry(&context).unwrap();
            let mut broker = registry.registry.brokers["recipient"].clone();
            broker.session_id = second.into();
            broker.incarnation = second_incarnation.into();
            registry.registry.brokers.insert(second.into(), broker);
            let seed = registry.registry.messages[0].clone();
            for _ in 1..256 {
                registry
                    .registry
                    .messages
                    .push(super::super::super::mailbox::StoredMessage {
                        message_id: uuid::Uuid::new_v4().to_string(),
                        ..seed.clone()
                    });
            }
            registry.save().unwrap();
        }
        {
            let mut journal = super::super::lock_journal(&context).unwrap();
            for index in 0..super::super::MAX_RETAINED_IDS {
                let mut item = own_outbox(&format!("full-backlog-{index}"), "rejected");
                item.state_changed_at_epoch = Some(now_epoch());
                item.reason = Some("remote-messaging-unsupported".into());
                if index == super::super::MAX_RETAINED_IDS - 1 {
                    item.envelope.from = Origin::Session(Address {
                        machine: config().machine,
                        session_id: second.into(),
                        session_incarnation: second_incarnation.into(),
                    });
                }
                journal.registry.retained.push(item.identity());
            }
            journal.save().unwrap();
        }
        let mut state = State::default();
        let retry = state
            .reconcile(&context, &config())
            .unwrap()
            .expect("blocked sender retry");
        let finished = now_epoch();
        assert!(
            retry > finished,
            "quota backoff must remain in the future after completed work"
        );
        assert_eq!(
            state.receive_attempts, 2,
            "one quota refusal defers that sender's entire backlog"
        );
        let registry = super::super::super::lock_registry(&context).unwrap();
        assert_eq!(
            registry
                .registry
                .messages
                .iter()
                .filter(|item| item.recipient_session_id == second)
                .count(),
            1
        );
        assert_eq!(
            registry
                .registry
                .messages
                .iter()
                .filter(|item| item.recipient_session_id == "recipient")
                .count(),
            256
        );
    }
}
