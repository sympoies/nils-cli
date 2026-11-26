use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, de::IgnoredAny};

use crate::cli::resume_move::{Provider, canonical_uuid};

pub(crate) const START_SCHEMA: &str = "agent-session.console-resume-move.v1";
pub(crate) const STATUS_SCHEMA: &str = "agent-session.console-resume-move-status.v1";
const BINDING_SCHEMA: &str = "agent-session.console-resume-move-binding.v1";
pub(crate) const BINDING_HEADER: &str = "x-agent-session-resume-move-binding";

pub(super) const ALLOWLIST: &[(&str, &str)] = &[
    ("resume-unsupported", "inspect_destination"),
    ("resume-account-mismatch", "inspect_destination"),
    ("resume-history-missing", "inspect_destination"),
    ("session-exists", "inspect_destination"),
    ("resume-source-live", "inspect_source"),
    ("resume-source-unknown", "inspect_source"),
    ("resume-move-source-fenced", "inspect_source"),
    ("resume-move-source-busy", "inspect_source"),
    ("resume-move-session-mismatch", "inspect_source"),
    ("resume-move-source-already-stopped", "inspect_source"),
    ("resume-move-proof-invalid", "inspect_destination"),
    ("resume-move-proof-expired", "inspect_destination"),
    ("resume-move-proof-missing", "inspect_destination"),
    ("resume-move-binding-mismatch", "inspect_destination"),
    ("resume-move-target-mismatch", "inspect_destination"),
    ("resume-move-path-mismatch", "inspect_destination"),
    (
        "resume-move-transcript-cwd-outside-checkout",
        "inspect_destination",
    ),
    ("resume-move-history-stale", "inspect_destination"),
    ("resume-move-history-unsupported", "inspect_destination"),
    ("resume-move-destination-reserved", "inspect_destination"),
    ("resume-move-ssh-keygen-unavailable", "inspect_destination"),
    ("resume-move-trust-unavailable", "inspect_destination"),
    ("resume-move-signing-unavailable", "inspect_destination"),
    ("resume-move-state-unavailable", "inspect_destination"),
    ("resume-move-unavailable", "inspect_destination"),
    (
        "resume-move-destination-writer-present",
        "reconcile_destination",
    ),
    ("resume-move-outcome-unknown", "reconcile_destination"),
    ("resume-move-aborted", "obtain_no_writer_receipt"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartRequest {
    schema_version: String,
    operation_id: String,
    machine: String,
    caller: Caller,
    pub(crate) session: Session,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Caller {
    session_id: String,
    session_incarnation: String,
    // The erratum assigns this particular forbidden member a binding refusal.
    #[serde(default, deserialize_with = "member_present")]
    owner: bool,
}

fn member_present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    IgnoredAny::deserialize(deserializer).map(|_| true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    pub(crate) move_id: String,
    pub(crate) agent: Provider,
    pub(crate) resume: String,
    pub(crate) account: String,
    pub(crate) cwd: PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) enum StartShape {
    Unsupported,
    OwnerAssertion,
}

impl StartRequest {
    pub(crate) fn parse(body: &[u8]) -> Result<Self, StartShape> {
        let request: Self = serde_json::from_slice(body).map_err(|_| StartShape::Unsupported)?;
        if request.schema_version != START_SCHEMA
            || !canonical_operation(&request.operation_id)
            || request.machine.trim().is_empty()
            || crate::validate_id(&request.caller.session_id).is_err()
            || !canonical_uuid(&request.caller.session_incarnation)
            || !canonical_uuid(&request.session.move_id)
            || !canonical_uuid(&request.session.resume)
            || request.session.account.trim().is_empty()
            || !Path::is_absolute(&request.session.cwd)
        {
            return Err(StartShape::Unsupported);
        }
        if request.caller.owner {
            return Err(StartShape::OwnerAssertion);
        }
        Ok(request)
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Scope {
    Read,
    Start,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    schema_version: String,
    origin_machine: String,
    caller_session_id: String,
    caller_session_incarnation: String,
    owner: String,
    destination_machine: String,
    move_id: String,
    scope: Scope,
    #[serde(default, deserialize_with = "present_string")]
    operation_id: Option<String>,
}

fn present_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl Binding {
    pub(crate) fn decode(header: &str) -> Option<Self> {
        if header.len() > 2 * 1024 {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(header).ok()?;
        let binding: Self = serde_json::from_slice(&bytes).ok()?;
        (binding.schema_version == BINDING_SCHEMA
            && !binding.origin_machine.trim().is_empty()
            && crate::validate_id(&binding.caller_session_id).is_ok()
            && canonical_uuid(&binding.caller_session_incarnation)
            && !binding.owner.trim().is_empty()
            && canonical_uuid(&binding.move_id))
        .then_some(binding)
    }

    pub(crate) fn matches_start(&self, machine: &str, request: &StartRequest) -> bool {
        self.scope == Scope::Start
            && self.destination_machine == machine
            && self.destination_machine == request.machine
            && self.move_id == request.session.move_id
            && self.caller_session_id == request.caller.session_id
            && self.caller_session_incarnation == request.caller.session_incarnation
            && self.operation_id.as_deref() == Some(request.operation_id.as_str())
    }

    pub(crate) fn matches_read(&self, machine: &str, move_id: &str) -> bool {
        self.scope == Scope::Read
            && self.destination_machine == machine
            && self.move_id == move_id
            && self.operation_id.is_none()
    }
}

fn canonical_operation(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

#[cfg(test)]
#[test]
fn console_resume_move_allowlist_matches_frozen_spec() {
    use std::collections::BTreeMap;

    let spec = include_str!("../../../docs/specs/console-resume-move-v1.md");
    let table = spec
        .split_once("| Literal codes |")
        .unwrap()
        .1
        .split_once("\n\n")
        .unwrap()
        .0;
    let mut expected = BTreeMap::new();
    for line in table.lines().filter(|line| line.starts_with("| `")) {
        let mut cells = line.split('|').skip(1);
        let codes = cells.next().unwrap();
        let action = cells.next().unwrap().trim().trim_matches('`');
        for code in codes.split(',') {
            assert!(
                expected
                    .insert(code.trim().trim_matches('`'), action)
                    .is_none()
            );
        }
    }
    let actual = ALLOWLIST.iter().copied().collect::<BTreeMap<_, _>>();
    assert_eq!(actual.len(), ALLOWLIST.len());
    pretty_assertions::assert_eq!(actual, expected);
}
