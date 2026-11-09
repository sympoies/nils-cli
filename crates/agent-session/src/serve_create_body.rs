use serde::Deserialize;
use serde_json::Value;

use super::{CreateTitleInput, SessionTitleStateInput};
use crate::CliError;
use crate::cli;

#[cfg(test)]
#[path = "serve/resume_tests.rs"]
mod resume_tests;

#[derive(Debug, Deserialize)]
pub(super) struct CreateBody {
    // Decode new metadata in the handler so malformed shapes use its envelope.
    #[serde(default = "default_origin")]
    origin: Value,
    #[serde(default)]
    workspace: Value,
    #[serde(default)]
    pub(super) launch_env: crate::launch_env::LaunchEnv,
    pub(super) agent: String,
    #[serde(default)]
    pub(super) title_mode: crate::display_metadata::TitleMode,
    #[serde(default)]
    pub(super) coordination_mode: cli::CoordinationMode,
    pub(super) agent_profile: Option<String>,
    pub(super) cwd: Option<String>,
    #[serde(default, deserialize_with = "super::deserialize_create_title")]
    pub(super) title: CreateTitleInput,
    pub(super) title_state: Option<SessionTitleStateInput>,
    pub(super) id: Option<String>,
    pub(super) prompt: Option<String>,
    #[serde(default, alias = "resume_id")]
    pub(super) provider_resume_id: Option<String>,
    /// Account-bound new-record resume; accepted only by the versioned resume route.
    #[serde(default)]
    pub(super) resume: Option<String>,
    /// Original account-selection consent; the resolved account remains a separate pin.
    #[serde(
        default,
        deserialize_with = "super::resume_start::deserialize_account_consent"
    )]
    pub(super) resume_account_explicit: Option<bool>,
    #[serde(default)]
    pub(super) agent_args: Vec<String>,
    #[serde(default)]
    pub(super) codex_account: Option<String>,
    #[serde(default)]
    pub(super) account: Option<String>,
    #[serde(default)]
    pub(super) claude_account: Option<String>,
    /// `session-lineage-work-v1`: who started this session, as the caller
    /// (Agent Console) states it. Absent: an operator root over HTTP.
    #[serde(default)]
    pub(super) lineage: Option<Value>,
    /// `session-lineage-work-v1`: resolved program and issue references.
    #[serde(default)]
    pub(super) work: Option<Value>,
    /// `session-lineage-work-v1`: configured launch role, stored verbatim.
    #[serde(default)]
    pub(super) role: Option<Value>,
}

/// Preserve key presence while decoding account fields; remove with #3468.
pub(super) struct CheckedCreateBody(pub(super) CreateBody);

impl<'de> Deserialize<'de> for CheckedCreateBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if let Some(object) = value.as_object()
            && ["account", "codex_account", "claude_account"]
                .iter()
                .any(|key| object.get(*key).is_some_and(Value::is_null))
        {
            let mut fields = object.clone();
            crate::coordination::resume_preflight::normalize_account(&mut fields)
                .map_err(|_| serde::de::Error::custom("conflicting account fields"))?;
        }
        serde_json::from_value(value)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

fn default_origin() -> Value {
    serde_json::json!({"parent":null})
}

impl CreateBody {
    /// Translate the prior resume wire fields during the #3468 transition.
    pub(super) fn normalize_resume_account(&mut self) -> Result<(), CliError> {
        let codex = self.codex_account.take();
        let claude = self.claude_account.take();
        if codex.is_some() && claude.is_some()
            || self.account.is_some() && (codex.is_some() || claude.is_some())
            || codex.is_some() && self.agent != "codex"
            || claude.is_some() && self.agent != "claude"
        {
            return Err(crate::start_resume::refusal("resume-unsupported"));
        }
        self.account = self.account.take().or(codex).or(claude);
        Ok(())
    }

    pub(super) fn operation(&self) -> &'static str {
        if self.provider_resume_id.is_some() {
            "import"
        } else {
            "create"
        }
    }

    pub(super) fn origin_fields(&self) -> Result<crate::origin::Fields, CliError> {
        if !self.origin.is_object()
            || self
                .origin
                .get("parent")
                .is_some_and(|parent| !parent.is_null() && !parent.is_object())
        {
            return Err(crate::origin::invalid_origin());
        }
        let fields = crate::origin::Fields {
            origin: serde_json::from_value(self.origin.clone())
                .map_err(|_| crate::origin::invalid_origin())?,
            workspace: serde_json::from_value(self.workspace.clone())
                .map_err(|_| crate::origin::invalid_workspace())?,
        };
        fields.validate()?;
        Ok(fields)
    }
}

/// The lineage and work a create body states, validated for a session on
/// `machine`. Without `lineage` the session is an operator root over HTTP.
pub(super) fn create_body_lineage(
    machine: &str,
    lineage: Option<&Value>,
    work: Option<&Value>,
    role: Option<&Value>,
) -> Result<crate::InitialLineage, CliError> {
    let seed = match lineage.filter(|value| !value.is_null()) {
        Some(value) => crate::lineage::LineageSeed::from_create_json(machine, value)?,
        None => crate::lineage::LineageSeed::root(
            machine,
            crate::lineage::STARTER_OPERATOR,
            crate::lineage::VIA_HTTP,
        ),
    };
    let work = match work.filter(|value| !value.is_null()) {
        Some(value) => crate::lineage::work_from_create_body(value)?,
        None => None,
    };
    let role = crate::lineage::role_from_create_body(role)?;
    crate::lineage::require_root_for_role(role.as_deref(), seed.has_parent())?;
    Ok(crate::InitialLineage {
        seed,
        work,
        role,
        origin: crate::origin::Fields::default(),
    })
}

/// Conflict rules for the per-provider account fields of a create body, in
/// one place for both providers:
///
/// - `<provider>_account` is accepted only for that provider's sessions;
/// - `provider_resume_id` imports run on the host login, so neither account
///   field may accompany one;
/// - Claude only, intentionally: a Claude `agent_profile` owns the provider
///   config root (`CLAUDE_CONFIG_DIR`) that an account binding would replace,
///   so `claude_account` cannot be combined with it. Codex credentials are
///   injected over its app-server control plane instead and do not conflict.
pub(super) fn validate_create_accounts(
    agent: cli::AgentKind,
    codex_account: bool,
    claude_account: bool,
    agent_profile: bool,
    provider_resume: bool,
) -> Result<(), CliError> {
    let requested = [
        (
            cli::AgentKind::Codex,
            codex_account,
            "codex",
            "Codex",
            "create the resumable Codex session first, then bind its account",
        ),
        (
            cli::AgentKind::Claude,
            claude_account,
            "claude",
            "Claude",
            "provider_resume_id imports run on the host Claude login; omit claude_account",
        ),
    ];
    for (owner, requested, provider, display, _) in requested {
        if requested && agent != owner {
            return Err(CliError::usage(
                format!("{provider}-account-agent-conflict"),
                format!("{provider}_account is supported only for {display} sessions"),
                None,
            ));
        }
    }
    if claude_account && agent_profile {
        return Err(CliError::usage(
            "claude-account-profile-conflict",
            "claude_account cannot be combined with an agent_profile",
            None,
        ));
    }
    if provider_resume {
        for (_, requested, provider, _, message) in requested {
            if requested {
                return Err(CliError::usage(
                    format!("{provider}-account-provider-resume-conflict"),
                    message,
                    None,
                ));
            }
        }
    }
    Ok(())
}

/// Capabilities of the daemon session list and its related routes.
pub(super) fn session_capabilities() -> Value {
    serde_json::json!({
        "profile_resume_import": true,
        "managed_resume_command": false,
        "resume_blocked_reason": true,
        "last_prompt": true,
        "last_message": true,
        "history": true,
        "archive": true,
        "group_archive": true,
        "history_star": true,
        "managed_account_handoff": crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY,
        "claude_account_switch": crate::claude_account::broker_is_configured(),
        "codex_account_switch": crate::codex_account::broker_is_configured(),
        "session_retitle_v2": true,
        "session_retitle_v3": true,
        "session_retitle_v3_receipts": true,
        "session_account_field": true,
    })
}
