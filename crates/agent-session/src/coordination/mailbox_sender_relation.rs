//! Descriptive sender lineage and recipient guidance; never an admission decision.
use std::collections::BTreeSet;

use nils_common::mailbox_notice::{
    DISPATCHER_GUIDANCE, PROMPT_PATH_PREFIX, PROMPT_PATH_SUFFIX, VERIFY_GUIDANCE,
};
use serde::Serialize;

use super::mailbox::StoredMessage;
use crate::lineage::{LINEAGE_SCHEMA, MAX_DEPTH, SessionRef};
use crate::{CliContext, SessionRecord, load_session_record};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Relation {
    Parent,
    Ancestor,
    Peer,
}

impl Relation {
    fn label(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Ancestor => "ancestor",
            Self::Peer => "peer",
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct View {
    sender_relation: Relation,
    guidance: String,
}

/// Read one bounded chain per projection, without remote I/O or prefix matching.
#[derive(Default)]
pub(super) struct SenderRelations {
    machine: String,
    ancestors: Vec<SessionRef>,
    prompt_file: Option<String>,
}

impl SenderRelations {
    pub(super) fn new(context: &CliContext, recipient: &SessionRecord) -> Self {
        let mut result = Self {
            prompt_file: recipient.prompt_file.clone(),
            ..Self::default()
        };
        let Some(mut lineage) = recipient.lineage.clone() else {
            return result;
        };
        if lineage.schema_version != LINEAGE_SCHEMA
            || lineage.machine.is_empty()
            || lineage.depth > MAX_DEPTH
        {
            return result;
        }
        result.machine = lineage.machine.clone();
        for _ in 0..lineage.depth {
            let Some(parent) = lineage.parent.clone() else {
                break;
            };
            if parent.machine != result.machine {
                // The recorded remote parent and root are known identities;
                // undiscovered intermediate remote ancestors remain peers.
                result.ancestors.push(parent.clone());
                if lineage.depth > 1 && !same_identity(&parent, &lineage.root) {
                    result.ancestors.push(lineage.root.clone());
                }
                break;
            }
            let Ok(record) = load_session_record(context, &parent.session_id) else {
                break;
            };
            if record.id != parent.session_id || record.created_at != parent.session_created_at {
                break;
            }
            let Some(parent_lineage) = record.lineage else {
                break;
            };
            if parent_lineage.schema_version != LINEAGE_SCHEMA
                || parent_lineage.machine != parent.machine
                || parent_lineage.depth.checked_add(1) != Some(lineage.depth)
                || !same_identity(&parent_lineage.root, &lineage.root)
            {
                break;
            }
            result.ancestors.push(parent);
            lineage = parent_lineage;
        }
        result
    }

    fn relation(&self, message: &StoredMessage) -> Relation {
        let remote = super::remote::sender_address(message);
        // Ingress machine names are claimed; real local senders are not remote-encoded.
        if remote
            .as_ref()
            .is_some_and(|address| address.machine == self.machine)
        {
            return Relation::Peer;
        }
        let (machine, id) = remote.as_ref().map_or(
            (self.machine.as_str(), message.sender_session_id.as_str()),
            |address| (address.machine.as_str(), address.session_id.as_str()),
        );
        match self
            .ancestors
            .iter()
            .position(|reference| reference.machine == machine && reference.session_id == id)
        {
            Some(0) => Relation::Parent,
            Some(_) => Relation::Ancestor,
            None => Relation::Peer,
        }
    }

    pub(super) fn view(&self, message: &StoredMessage) -> View {
        let sender_relation = self.relation(message);
        View {
            sender_relation,
            guidance: self.guidance(sender_relation != Relation::Peer),
        }
    }

    pub(super) fn inbox_view(&self, message: &StoredMessage) -> View {
        let sender_relation = self.relation(message);
        View {
            sender_relation,
            guidance: if sender_relation == Relation::Peer {
                String::new()
            } else {
                self.dispatcher_guidance()
            },
        }
    }

    pub(super) fn notice_guidance(&self, messages: &[&StoredMessage]) -> String {
        let mut relations: BTreeSet<_> = messages.iter().map(|m| self.relation(m)).collect();
        if relations.is_empty() {
            relations.insert(Relation::Peer);
        }
        let labels = relations
            .iter()
            .map(|r| format!("sender_relation={}", r.label()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "{labels}\n{}",
            self.guidance(relations.iter().any(|r| *r != Relation::Peer))
        )
    }

    fn guidance(&self, dispatching: bool) -> String {
        let mut guidance = VERIFY_GUIDANCE.to_string();
        if dispatching {
            guidance.push('\n');
            guidance.push_str(&self.dispatcher_guidance());
        }
        guidance
    }

    fn dispatcher_guidance(&self) -> String {
        let mut guidance = DISPATCHER_GUIDANCE.to_string();
        if let Some(path) = &self.prompt_file {
            guidance.push_str(&format!("{PROMPT_PATH_PREFIX}{path}{PROMPT_PATH_SUFFIX}"));
        }
        guidance.push('.');
        guidance
    }
}

fn same_identity(left: &SessionRef, right: &SessionRef) -> bool {
    left.machine == right.machine
        && left.session_id == right.session_id
        && left.session_created_at == right.session_created_at
}

pub(super) fn inbox_guidance() -> String {
    format!(
        "Mailbox reminder dates bound the newest queued message. If you already read the inbox after that time, nothing new is waiting.\n{VERIFY_GUIDANCE}\nTo find new mail, run {} message inbox --state unread. Keep any explicit --session and authentication options. Repeat with --cursor and the same filters while next_cursor is present. Use message show to read listed messages. Full guidance is read from the inbox.",
        crate::executable()
    )
}
