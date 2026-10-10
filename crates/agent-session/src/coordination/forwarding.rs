//! Recipient-owned forwarding: immutable data and bounded, untrusted provenance.
use serde::{Deserialize, Serialize};

use super::{
    Registry,
    mailbox::StoredMessage,
    remote::{Address, Origin},
};
use crate::{CliError, cli::MessageCategory};

pub(super) const MAX_HOPS: usize = 8;
pub(super) const MAX_RECIPIENT_TRANSFERS: usize = 8;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub message: String,
    pub if_revision: u64,
    #[serde(default)]
    pub categories: Vec<MessageCategory>,
}
impl Request {
    pub fn normalize(&mut self) {
        self.categories.sort_unstable();
        self.categories.dedup();
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    /// Original identity is attested by the forwarder, not authenticated end-to-end.
    pub attestation: String,
    pub original_message_id: String,
    pub original_sender: Origin,
    pub original_recipient: Address,
    pub original_created_at_epoch: i64,
    pub original_expires_at_epoch: i64,
    pub body_sha256: String,
    pub hops: Vec<Hop>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hop {
    pub source_message_id: String,
    pub source_revision: u64,
    pub forwarder: Address,
    pub recipient: Address,
    pub forwarded_at_epoch: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipient_transfers: Vec<RecipientTransfer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecipientTransfer {
    pub message_id: String,
    pub from: Address,
    pub to: Address,
    pub controller: Address,
    pub source_revision: u64,
    pub transferred_at_epoch: i64,
}

pub(super) fn source<'a>(
    registry: &'a Registry,
    actor: &Address,
    request: &Request,
    now: i64,
) -> Result<&'a StoredMessage, CliError> {
    let message = registry
        .messages
        .iter()
        .find(|message| {
            message.message_id == request.message
                && message.recipient_session_id == actor.session_id
                && message.recipient_incarnation == actor.session_incarnation
        })
        .ok_or_else(|| {
            CliError::data("message-not-found", "received message does not exist", None)
        })?;
    if matches!(message.state.as_str(), "expired" | "quarantined")
        || message.expires_at_epoch <= now
    {
        return Err(CliError::data(
            "message-expired",
            "forward source is unavailable",
            None,
        ));
    }
    if message.revision != request.if_revision {
        return Err(CliError::data(
            "message-revision-conflict",
            "forward source revision changed",
            None,
        ));
    }
    if !request.categories.is_empty()
        && !request
            .categories
            .contains(&message.category.unwrap_or_default())
    {
        return Err(CliError::data(
            "message-category-conflict",
            "forward source category does not match",
            None,
        ));
    }
    if message.forwarding.as_ref().is_some_and(|provenance| {
        provenance
            .hops
            .last()
            .and_then(|hop| hop.recipient_transfers.last())
            .is_some_and(|transfer| {
                transfer.message_id != message.message_id
                    || transfer.source_revision >= message.revision
            })
    }) {
        return Err(invalid());
    }
    Ok(message)
}

fn invalid() -> CliError {
    CliError::data(
        "message-forward-invalid",
        "forward provenance is invalid",
        None,
    )
}

/// Called only under the registry lock and the controller's authorized carry guard.
/// Historical hop endpoints stay immutable; this records the local receiver transfer.
pub(super) fn record_recipient_transfer(
    message: &mut StoredMessage,
    machine: &str,
    current_incarnation: &str,
    controller: Address,
    now: i64,
) -> Result<(), CliError> {
    let Some(existing) = &message.forwarding else {
        return Ok(());
    };
    let from = Address {
        machine: machine.into(),
        session_id: message.recipient_session_id.clone(),
        session_incarnation: message.recipient_incarnation.clone(),
    };
    let to = Address {
        session_incarnation: current_incarnation.into(),
        ..from.clone()
    };
    if from == to {
        return Ok(());
    }
    if !existing.valid(&from, &message.body, message.expires_at_epoch, now)
        || existing
            .hops
            .iter()
            .map(|hop| hop.recipient_transfers.len())
            .sum::<usize>()
            >= MAX_RECIPIENT_TRANSFERS
        || existing.hops.last().is_none_or(|hop| {
            hop.forwarder != controller
                || hop
                    .recipient_transfers
                    .last()
                    .is_some_and(|t| t.message_id != message.message_id)
        })
    {
        return Err(invalid());
    }
    let mut audit = existing.clone();
    audit
        .hops
        .last_mut()
        .ok_or_else(invalid)?
        .recipient_transfers
        .push(RecipientTransfer {
            message_id: message.message_id.clone(),
            from,
            to: to.clone(),
            controller,
            source_revision: message.revision,
            transferred_at_epoch: now,
        });
    if !audit.valid(&to, &message.body, message.expires_at_epoch, now) {
        return Err(invalid());
    }
    message.forwarding = Some(audit);
    Ok(())
}

/// Called only under the registry lock while a verified resume replaces the
/// exact predecessor incarnation. The predecessor recipient attests its own
/// transfer, so `controller` equals `from`; no forwarder authority is claimed.
pub(super) fn record_resume_transfer(
    message: &mut StoredMessage,
    machine: &str,
    current_incarnation: &str,
    now: i64,
) -> Result<(), CliError> {
    let Some(existing) = &message.forwarding else {
        return Ok(());
    };
    let from = Address {
        machine: machine.into(),
        session_id: message.recipient_session_id.clone(),
        session_incarnation: message.recipient_incarnation.clone(),
    };
    let to = Address {
        session_incarnation: current_incarnation.into(),
        ..from.clone()
    };
    if from == to
        || !existing.valid(&from, &message.body, message.expires_at_epoch, now)
        || existing
            .hops
            .iter()
            .map(|hop| hop.recipient_transfers.len())
            .sum::<usize>()
            >= MAX_RECIPIENT_TRANSFERS
        || existing.hops.last().is_none_or(|hop| {
            hop.recipient_transfers
                .last()
                .is_some_and(|t| t.message_id != message.message_id)
        })
    {
        return Err(invalid());
    }
    let mut audit = existing.clone();
    audit
        .hops
        .last_mut()
        .ok_or_else(invalid)?
        .recipient_transfers
        .push(RecipientTransfer {
            message_id: message.message_id.clone(),
            from: from.clone(),
            to: to.clone(),
            controller: from,
            source_revision: message.revision,
            transferred_at_epoch: now,
        });
    if !audit.valid(&to, &message.body, message.expires_at_epoch, now) {
        return Err(invalid());
    }
    message.forwarding = Some(audit);
    Ok(())
}

fn same_session(left: &Address, right: &Address) -> bool {
    left.machine == right.machine && left.session_id == right.session_id
}

pub(super) fn append(
    message: &StoredMessage,
    actor: Address,
    destination: Address,
    now: i64,
) -> Result<Provenance, CliError> {
    let mut provenance = message.forwarding.clone().unwrap_or_else(|| Provenance {
        attestation: "forwarder".into(),
        original_message_id: message.message_id.clone(),
        original_sender: super::mailbox::sender_origin(message, &actor.machine),
        original_recipient: actor.clone(),
        original_created_at_epoch: message
            .remote_created_at_epoch
            .unwrap_or(message.created_at_epoch),
        original_expires_at_epoch: message.expires_at_epoch,
        body_sha256: super::digest_bytes(message.body.as_bytes()),
        hops: Vec::new(),
    });
    if provenance.hops.len() >= MAX_HOPS {
        return Err(CliError::data(
            "message-forward-depth-exceeded",
            "forward hop limit exceeded",
            None,
        ));
    }
    if same_session(&actor, &destination)
        || same_session(&provenance.original_recipient, &destination)
        || matches!(&provenance.original_sender, Origin::Session(sender) if same_session(sender, &destination))
        || provenance
            .hops
            .iter()
            .any(|hop| same_session(&hop.recipient, &destination))
    {
        return Err(CliError::data(
            "message-forward-loop",
            "forward destination is a prior participant",
            None,
        ));
    }
    provenance.hops.push(Hop {
        source_message_id: message.message_id.clone(),
        source_revision: message.revision,
        forwarder: actor,
        recipient: destination.clone(),
        forwarded_at_epoch: now,
        recipient_transfers: Vec::new(),
    });
    if !provenance.valid(&destination, &message.body, message.expires_at_epoch, now) {
        return Err(CliError::data(
            "message-forward-invalid",
            "forward provenance is invalid",
            None,
        ));
    }
    Ok(provenance)
}

impl Provenance {
    pub fn valid(&self, destination: &Address, body: &str, expiry: i64, now: i64) -> bool {
        if self.attestation != "forwarder"
            || !self.original_sender.valid()
            || !valid_address(&self.original_recipient)
            || uuid::Uuid::parse_str(&self.original_message_id).is_err()
            || self.body_sha256 != super::digest_bytes(body.as_bytes())
            || self.original_expires_at_epoch <= self.original_created_at_epoch
            || self
                .original_expires_at_epoch
                .saturating_sub(self.original_created_at_epoch)
                > super::mailbox::MAX_EXPIRY_SECS
            || expiry > self.original_expires_at_epoch
            || self.hops.is_empty()
            || self.hops.len() > MAX_HOPS
            || self
                .hops
                .iter()
                .map(|hop| hop.recipient_transfers.len())
                .sum::<usize>()
                > MAX_RECIPIENT_TRANSFERS
        {
            return false;
        }
        let mut previous = &self.original_recipient;
        let mut participants = vec![previous];
        if let Origin::Session(sender) = &self.original_sender {
            participants.push(sender);
        }
        let mut previous_time = self.original_created_at_epoch;
        let mut prior_transfer: Option<&RecipientTransfer> = None;
        for (index, hop) in self.hops.iter().enumerate() {
            if &hop.forwarder != previous
                || prior_transfer.is_some_and(|transfer| {
                    transfer.message_id != hop.source_message_id
                        || transfer.source_revision >= hop.source_revision
                })
                || !valid_address(&hop.forwarder)
                || !valid_address(&hop.recipient)
                || hop.source_revision == 0
                || uuid::Uuid::parse_str(&hop.source_message_id).is_err()
                || (index == 0 && hop.source_message_id != self.original_message_id)
                || hop.forwarded_at_epoch < previous_time
                || hop.forwarded_at_epoch > now.saturating_add(60)
                || participants
                    .iter()
                    .any(|participant| same_session(participant, &hop.recipient))
            {
                return false;
            }
            participants.push(&hop.recipient);
            previous = &hop.recipient;
            previous_time = hop.forwarded_at_epoch;
            prior_transfer = None;
            for transfer in &hop.recipient_transfers {
                if &transfer.from != previous
                    || !valid_address(&transfer.from)
                    || !valid_address(&transfer.to)
                    || !valid_address(&transfer.controller)
                    || !same_session(&transfer.from, &transfer.to)
                    || transfer.from.session_incarnation == transfer.to.session_incarnation
                    // A controller carry is attested by the hop forwarder; a
                    // same-session resume carry by its exact predecessor.
                    || (transfer.controller != hop.forwarder && transfer.controller != transfer.from)
                    || transfer.controller.machine != transfer.from.machine
                    || uuid::Uuid::parse_str(&transfer.message_id).is_err()
                    || transfer.source_revision == 0
                    || prior_transfer.is_some_and(|prior| {
                        prior.message_id != transfer.message_id
                            || prior.source_revision >= transfer.source_revision
                    })
                    || transfer.transferred_at_epoch < previous_time
                    || transfer.transferred_at_epoch > now.saturating_add(60)
                {
                    return false;
                }
                previous = &transfer.to;
                previous_time = transfer.transferred_at_epoch;
                prior_transfer = Some(transfer);
            }
        }
        previous == destination
    }
}
fn valid_address(address: &Address) -> bool {
    Origin::Session(address.clone()).valid()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn address(session: &str) -> Address {
        Address {
            machine: "fixture".into(),
            session_id: session.into(),
            session_incarnation: format!("incarnation-{session}"),
        }
    }
    fn message() -> StoredMessage {
        serde_json::from_value(serde_json::json!({
            "schema_version": "agent-session.message.v1", "message_id": uuid::Uuid::new_v4().to_string(),
            "sender_session_id": "worker", "sender_incarnation": "incarnation-worker",
            "recipient_session_id": "coordinator", "recipient_incarnation": "incarnation-coordinator",
            "state": "unread", "revision": 1, "reply_to": null, "reply_depth": 0,
            "created_at": "", "created_at_epoch": 100, "expires_at": "", "expires_at_epoch": 1000,
            "body_bytes": 4, "body": "body"
        })).unwrap()
    }

    #[test]
    fn controller_recipient_transfers_bind_copy_revision_identity_and_time() {
        let controller = address("coordinator");
        let worker = address("receiver");
        let mut carried = message();
        let audit = append(&carried, controller.clone(), worker.clone(), 101).unwrap();
        assert!(
            serde_json::to_value(&audit).unwrap()["hops"][0]
                .get("recipient_transfers")
                .is_none()
        );
        carried.message_id = uuid::Uuid::new_v4().to_string();
        carried.recipient_session_id = worker.session_id.clone();
        carried.recipient_incarnation = worker.session_incarnation.clone();
        carried.forwarding = Some(audit);
        for index in 1..=MAX_RECIPIENT_TRANSFERS {
            let incarnation = format!("resumed-{index}");
            record_recipient_transfer(
                &mut carried,
                "fixture",
                &incarnation,
                controller.clone(),
                101 + index as i64,
            )
            .unwrap();
            carried.recipient_incarnation = incarnation;
            carried.revision += 1;
        }
        let actor = Address {
            session_incarnation: carried.recipient_incarnation.clone(),
            ..worker.clone()
        };
        let destination = address("target");
        let forwarded = append(&carried, actor, destination.clone(), 200).unwrap();
        assert_eq!(
            forwarded.hops[0].recipient, worker,
            "original hop is immutable"
        );
        assert_eq!(
            forwarded.hops[0].recipient_transfers.len(),
            MAX_RECIPIENT_TRANSFERS
        );
        assert!(forwarded.valid(&destination, "body", 1000, 200));
        assert_eq!(
            record_recipient_transfer(&mut carried, "fixture", "one-too-many", controller, 200)
                .unwrap_err()
                .code(),
            "message-forward-invalid"
        );
        for field in ["controller", "from", "to", "message_id", "revision", "time"] {
            let mut tampered = forwarded.clone();
            let transfer = &mut tampered.hops[0].recipient_transfers[0];
            match field {
                "controller" => transfer.controller.session_id = "unproven-controller".into(),
                "from" => transfer.from.session_incarnation = "unproven-incarnation".into(),
                "to" => transfer.to.session_id = "another-worker".into(),
                "message_id" => transfer.message_id = uuid::Uuid::new_v4().to_string(),
                "revision" => transfer.source_revision = 0,
                "time" => transfer.transferred_at_epoch = 100,
                _ => unreachable!(),
            }
            assert!(!tampered.valid(&destination, "body", 1000, 200), "{field}");
        }
        let mut wrong_next = forwarded.clone();
        wrong_next.hops[1].source_revision = MAX_RECIPIENT_TRANSFERS as u64;
        assert!(!wrong_next.valid(&destination, "body", 1000, 200));
        let mut value = serde_json::to_value(&forwarded).unwrap();
        value["hops"][1]["recipient_transfers"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<Provenance>(value).is_err());
    }

    #[test]
    fn resume_transfer_is_attested_by_the_exact_predecessor_and_stays_forwardable() {
        let coordinator = address("coordinator");
        let worker = address("receiver");
        let mut carried = message();
        let audit = append(&carried, coordinator, worker.clone(), 101).unwrap();
        carried.message_id = uuid::Uuid::new_v4().to_string();
        carried.recipient_session_id = worker.session_id.clone();
        carried.recipient_incarnation = worker.session_incarnation.clone();
        carried.forwarding = Some(audit);
        record_resume_transfer(&mut carried, "fixture", "resumed", 102).unwrap();
        let transfer = carried.forwarding.as_ref().unwrap().hops[0].recipient_transfers[0].clone();
        assert_eq!(transfer.controller, worker);
        assert_eq!(transfer.from, worker);
        assert_eq!(transfer.to.session_incarnation, "resumed");
        carried.recipient_incarnation = "resumed".into();
        carried.revision += 1;
        let actor = Address {
            session_incarnation: "resumed".into(),
            ..worker.clone()
        };
        let destination = address("target");
        let forwarded = append(&carried, actor, destination.clone(), 200).unwrap();
        assert!(forwarded.valid(&destination, "body", 1000, 200));
        let mut tampered = forwarded.clone();
        tampered.hops[0].recipient_transfers[0].controller = address("unrelated");
        assert!(!tampered.valid(&destination, "body", 1000, 200));
    }

    #[test]
    fn forward_hop_bound_and_reincarnation_loops_preserve_root_and_expiry() {
        let mut source = message();
        let root = source.message_id.clone();
        let mut actor = address("coordinator");
        for index in 1..=MAX_HOPS {
            let destination = address(&format!("recipient-{index}"));
            let audit = append(
                &source,
                actor.clone(),
                destination.clone(),
                100 + index as i64,
            )
            .unwrap();
            assert_eq!(audit.hops.len(), index);
            assert_eq!(audit.original_message_id, root);
            assert_eq!(audit.original_expires_at_epoch, 1000);
            assert!(audit.valid(&destination, "body", 1000, 200));
            assert!(!audit.valid(&destination, "changed", 1000, 200));
            assert!(!audit.valid(&destination, "body", 1001, 200));
            source.message_id = uuid::Uuid::new_v4().to_string();
            source.forwarding = Some(audit);
            source.recipient_session_id = destination.session_id.clone();
            source.recipient_incarnation = destination.session_incarnation.clone();
            actor = destination;
        }
        assert_eq!(
            append(&source, actor, address("next"), 200)
                .unwrap_err()
                .code(),
            "message-forward-depth-exceeded"
        );
        let original = message();
        let mut replaced_worker = address("worker");
        replaced_worker.session_incarnation = "replacement".into();
        assert_eq!(
            append(&original, address("coordinator"), replaced_worker, 200)
                .unwrap_err()
                .code(),
            "message-forward-loop"
        );
    }

    #[test]
    fn source_forward_preserves_category_and_service_authority_without_relabeling() {
        let mut source = message();
        source.category = Some(MessageCategory::Report);
        let service = super::super::service::Origin {
            machine: "fixture".into(),
            service_id: "automation".into(),
            service_generation: "generation-1".into(),
        };
        source.sender_session_id = service.stored_id();
        source.sender_incarnation = service.service_generation.clone();
        let audit = append(&source, address("coordinator"), address("steward"), 200).unwrap();
        assert!(matches!(audit.original_sender, Origin::Service(_)));
        assert_eq!(source.category, Some(MessageCategory::Report));
        assert_eq!(audit.attestation, "forwarder");
        let mut tampered = audit.clone();
        tampered.hops[0].forwarder = address("unrelated");
        assert!(!tampered.valid(&address("steward"), "body", 1000, 200));
    }
}
