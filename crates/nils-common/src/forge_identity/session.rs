//! Metadata-only launch context and authenticated session identity projection.
use super::{Error, Policy, Result, policy, probe};
use serde::{Deserialize, Serialize};
use std::process::Command;

pub const CONTEXT_KEY: &str = "forge_context";
pub const BINDING_SCHEMA: &str = "agent-session.forge-binding.v1";
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchContext {
    pub initiator: String,
    pub role: Option<String>,
}
impl LaunchContext {
    pub fn validate(&self) -> Result<()> {
        if !policy::identifier(&self.initiator)
            || self.role.as_ref().is_some_and(|r| !policy::identifier(r))
        {
            return Err(Error::new("identity_session_binding_invalid"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReference {
    pub machine: String,
    pub session_id: String,
    pub session_created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_incarnation: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    pub schema_version: String,
    pub session_id: String,
    pub session_incarnation: String,
    pub session_created_at: String,
    pub root: SessionReference,
    pub parent: Option<SessionReference>,
    pub initiator: String,
    pub role: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct SessionDecision {
    #[serde(flatten)]
    pub binding: SessionBinding,
    pub matched_launch_rule: String,
}
impl SessionBinding {
    pub fn validate(&self) -> Result<()> {
        let reference_valid = |r: &SessionReference| {
            policy::identifier(&r.machine)
                && policy::identifier(&r.session_id)
                && r.session_created_at.parse::<jiff::Timestamp>().is_ok()
                && r.session_incarnation
                    .as_ref()
                    .is_none_or(|i| policy::identifier(i))
        };
        if self.schema_version != BINDING_SCHEMA
            || !policy::identifier(&self.session_id)
            || !policy::identifier(&self.session_incarnation)
            || self.session_created_at.parse::<jiff::Timestamp>().is_err()
            || !reference_valid(&self.root)
            || self.parent.as_ref().is_some_and(|p| !reference_valid(p))
        {
            return Err(Error::new("identity_session_binding_invalid"));
        }
        LaunchContext {
            initiator: self.initiator.clone(),
            role: self.role.clone(),
        }
        .validate()
    }
}
/// Each invocation authenticates the live broker incarnation. Never interpret
/// an environment principal or a public board record as a session binding.
pub fn current() -> Result<SessionBinding> {
    let session = std::env::var("AGENT_SESSION_ID")
        .map_err(|_| Error::new("identity_session_binding_missing"))?;
    let incarnation = std::env::var("AGENT_SESSION_RUNTIME_ID")
        .map_err(|_| Error::new("identity_session_binding_missing"))?;
    if !policy::identifier(&session) || !policy::identifier(&incarnation) {
        return Err(Error::new("identity_session_binding_invalid"));
    }
    let executable = std::env::var_os("FORGE_IDENTITY_AGENT_SESSION_BIN")
        .unwrap_or_else(|| "agent-session".into());
    let mut cmd = Command::new(executable);
    cmd.args([
        "broker",
        "identity",
        "--session",
        &session,
        "--format",
        "json",
    ]);
    let out = probe::run(&mut cmd).map_err(|e| match e.code {
        "identity_probe_timeout" | "identity_probe_output_limit" => e,
        _ => Error::new("identity_session_binding_unavailable"),
    })?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| Error::new("identity_session_binding_unavailable"))?;
    if !out.status.success()
        || v["ok"] != true
        || v["schema_version"] != "cli.agent-session.broker-identity.v1"
    {
        return Err(Error::new("identity_session_binding_unavailable"));
    }
    let binding: SessionBinding = serde_json::from_value(v["data"].clone())
        .map_err(|_| Error::new("identity_session_binding_invalid"))?;
    binding.validate()?;
    if binding.session_id != session || binding.session_incarnation != incarnation {
        return Err(Error::new("identity_session_binding_mismatch"));
    }
    Ok(binding)
}
impl Policy {
    pub fn principal_for_launch(&self, context: &LaunchContext) -> Result<(String, String)> {
        context.validate()?;
        let matches: Vec<_> = self
            .launch_rules
            .iter()
            .filter(|r| {
                r.initiator == context.initiator && (r.role.is_none() || r.role == context.role)
            })
            .collect();
        let exact: Vec<_> = matches
            .iter()
            .copied()
            .filter(|r| r.role.is_some())
            .collect();
        let selected = if exact.is_empty() { &matches } else { &exact };
        match selected.as_slice() {
            [rule] => Ok((rule.principal.clone(), rule.id.clone())),
            [] => Err(Error::new("identity_launch_rule_missing")),
            _ => Err(Error::new("identity_launch_rule_ambiguous")),
        }
    }
    pub fn session_decision(&self, binding: SessionBinding) -> Result<(String, SessionDecision)> {
        binding.validate()?;
        let (principal, matched_launch_rule) = self.principal_for_launch(&LaunchContext {
            initiator: binding.initiator.clone(),
            role: binding.role.clone(),
        })?;
        Ok((
            principal,
            SessionDecision {
                binding,
                matched_launch_rule,
            },
        ))
    }
}
