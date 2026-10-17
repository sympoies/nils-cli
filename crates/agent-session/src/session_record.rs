use crate::{ProviderResume, RuntimeInfo, SessionTitleState, cli, lineage, metadata};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionRecord {
    pub schema_version: String,
    pub id: String,
    pub agent: String,
    pub mode: String,
    #[serde(default)]
    pub coordination_mode: cli::CoordinationMode,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_state: Option<SessionTitleState>,
    #[serde(default)]
    pub title_revision: u64,
    pub cwd: String,
    pub tmux_session: String,
    pub prompt_file: Option<String>,
    pub log_file: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_resume: Option<ProviderResume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_metadata: Option<metadata::SessionMetadataState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_bin: Option<String>,
    /// Who started this session (`session-lineage-work-v1`); written once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<lineage::SessionLineage>,
    /// Program and issue references (`session-lineage-work-v1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<lineage::SessionWork>,
    /// A later steward (`lineage adopt`); `lineage` itself never changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_adoption: Option<lineage::LineageAdoption>,
    /// `coordinator` when the session was started as one; written once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
    #[serde(skip)]
    pub resume_sidecar_extra: BTreeMap<String, Value>,
}
