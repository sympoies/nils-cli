use serde::Deserialize;
use serde_json::Value;

use super::{CreateTitleInput, SessionTitleStateInput};
use crate::cli;

#[derive(Debug, Deserialize)]
pub(super) struct CreateBody {
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
    #[serde(default)]
    pub(super) agent_args: Vec<String>,
    #[serde(default)]
    pub(super) codex_account: Option<String>,
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
