//! Supervisor role authority comes only from operator-attested broker provenance.
use nils_common::forge_identity::session;
use serde::{Deserialize, Serialize};

use crate::error::ForgeError;
use crate::ops::{
    pr_review_handoff::{fail, session_digest},
    review_state::sha256_digest,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SupervisorAttestation {
    pub role: String,
    /// Digest of the complete authenticated session/incarnation launch binding.
    pub binding_digest: String,
}

impl SupervisorAttestation {
    pub(crate) fn valid(&self) -> bool {
        session::LaunchContext {
            initiator: "operator".into(),
            role: Some(self.role.clone()),
            attested_role: Some(self.role.clone()),
        }
        .validate()
        .is_ok()
            && self
                .binding_digest
                .strip_prefix("sha256:")
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
    }
}

pub(super) fn authenticate(
    supervisors: &[String],
    actor: &str,
) -> Result<SupervisorAttestation, ForgeError> {
    let refused = || {
        fail(
            "review_writer_conflict",
            "takeover requires a repository-configured operator-attested supervisor role",
        )
    };
    // The broker checks the caller capability and current runtime incarnation.
    // Ordinary role labels and inherited initiators grant no supervisor authority.
    let binding = session::current().map_err(|_| refused())?;
    let role = binding.attested_role.as_deref().ok_or_else(refused)?;
    let selector = format!("role:{role}");
    if session_digest(&binding.session_id)? != actor || !supervisors.contains(&selector) {
        return Err(refused());
    }
    let bytes = serde_json::to_vec(&binding).map_err(|_| refused())?;
    Ok(SupervisorAttestation {
        role: role.into(),
        binding_digest: sha256_digest(&bytes),
    })
}
