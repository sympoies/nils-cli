//! Identifier-only evidence of lifecycle operations performed on a managed record.
use crate::SessionRecord;
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "agent-session.provider-continuity.v1";
const KEY: &str = "provider_continuity";
const MAX_LINKS: usize = 16;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Clear,
    Restart,
    Resume,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Link {
    kind: Kind,
    from_incarnation: String,
    to_incarnation: String,
    from_conversation: String,
    to_conversation: String,
    at_epoch: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderContinuity {
    schema_version: String,
    provider: String,
    links: Vec<Link>,
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn endpoint(record: &SessionRecord) -> Option<(&str, &str)> {
    let incarnation = &record.runtime.as_ref()?.launch_id;
    let resume = record.provider_resume.as_ref()?;
    (matches!(record.agent.as_str(), "claude" | "codex")
        && resume.provider == record.agent
        && valid_id(incarnation)
        && valid_id(&resume.session_id))
    .then_some((incarnation, &resume.session_id))
}

fn valid_link(link: &Link) -> bool {
    [
        &link.from_incarnation,
        &link.to_incarnation,
        &link.from_conversation,
        &link.to_conversation,
    ]
    .into_iter()
    .all(|id| valid_id(id))
        && link.at_epoch >= 0
        && match link.kind {
            Kind::Clear => link.from_conversation != link.to_conversation,
            Kind::Restart | Kind::Resume => link.from_conversation == link.to_conversation,
        }
}

fn retained(record: &SessionRecord) -> Option<ProviderContinuity> {
    let (incarnation, conversation) = endpoint(record)?;
    let proof: ProviderContinuity = serde_json::from_value(record.extra.get(KEY)?.clone()).ok()?;
    let last = proof.links.last()?;
    (proof.schema_version == SCHEMA
        && proof.provider == record.agent
        && proof.links.len() <= MAX_LINKS
        && proof.links.iter().all(valid_link)
        && proof.links.windows(2).all(|pair| {
            pair[0].to_incarnation == pair[1].from_incarnation
                && pair[0].to_conversation == pair[1].from_conversation
                && pair[0].at_epoch <= pair[1].at_epoch
        })
        && last.to_incarnation == incarnation
        && last.to_conversation == conversation)
        .then_some(proof)
}

pub(crate) fn projection(record: &SessionRecord, status: &str) -> Option<ProviderContinuity> {
    (status == "running").then(|| retained(record)).flatten()
}

/// Call only at an owned lifecycle mutation, under its record lock. Observing
/// or rebinding a provider identity never calls this function.
pub(crate) fn append(previous: &SessionRecord, next: &mut SessionRecord, kind: Kind) {
    let endpoints = endpoint(previous).zip(endpoint(next));
    let proof = endpoints.and_then(
        |((from_incarnation, from_conversation), (to_incarnation, to_conversation))| {
            if previous.id != next.id || previous.agent != next.agent {
                return None;
            }
            let mut proof = retained(previous).unwrap_or_else(|| ProviderContinuity {
                schema_version: SCHEMA.into(),
                provider: previous.agent.clone(),
                links: Vec::new(),
            });
            let link = Link {
                kind,
                from_incarnation: from_incarnation.into(),
                to_incarnation: to_incarnation.into(),
                from_conversation: from_conversation.into(),
                to_conversation: to_conversation.into(),
                at_epoch: jiff::Timestamp::now()
                    .as_second()
                    .max(proof.links.last().map_or(0, |link| link.at_epoch)),
            };
            if !valid_link(&link) {
                return None;
            }
            proof.links.push(link);
            let excess = proof.links.len().saturating_sub(MAX_LINKS);
            proof.links.drain(..excess);
            Some(proof)
        },
    );
    next.extra.remove(KEY);
    if let Some(proof) = proof {
        next.extra.insert(
            KEY.into(),
            serde_json::to_value(proof).expect("continuity serialization"),
        );
    }
}

#[cfg(test)]
#[path = "provider_continuity_contract_tests.rs"]
mod tests;
