use crate::{
    AgentKind, CliError, ProviderResume, RuntimeInfo, SessionTitleState, cli, lineage, metadata,
};
use jiff::Zoned;
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

const RESUME_MOVE_LINEAGE_SCHEMA: &str = "agent-session.resume-move-lineage.v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResumeMoveLineage {
    schema_version: String,
    move_id: String,
    source_host: String,
    source_session_id: String,
    source_incarnation: String,
    source_generation: u64,
    provider: cli::resume_move::Provider,
    provider_session_id: String,
}

impl ResumeMoveLineage {
    pub(crate) fn from_move(binding: &cli::resume_move::Binding, source_generation: u64) -> Self {
        Self {
            schema_version: RESUME_MOVE_LINEAGE_SCHEMA.into(),
            move_id: binding.move_id.clone(),
            source_host: binding.source_host.clone(),
            source_session_id: binding.source_session_id.clone(),
            source_incarnation: binding.source_incarnation.clone(),
            source_generation,
            provider: binding.provider,
            provider_session_id: binding.provider_session_id.clone(),
        }
    }
}

impl SessionRecord {
    pub(crate) fn store_resume_move(&mut self, lineage: ResumeMoveLineage) {
        self.extra
            .insert("resume_move".into(), serde_json::json!(lineage));
    }

    pub(crate) fn resume_move(&self) -> Option<ResumeMoveLineage> {
        let lineage: ResumeMoveLineage =
            serde_json::from_value(self.extra.get("resume_move")?.clone()).ok()?;
        (lineage.schema_version == RESUME_MOVE_LINEAGE_SCHEMA).then_some(lineage)
    }
}

#[derive(Debug, Default)]
pub(crate) struct InitialProviderPlan {
    pub(crate) provider_resume: Option<ProviderResume>,
    pub(crate) launch_args: Vec<String>,
    pub(crate) move_lineage: Option<ResumeMoveLineage>,
}

pub(crate) fn initial_provider_resume_plan(
    agent: AgentKind,
) -> Result<InitialProviderPlan, CliError> {
    let plan = match agent {
        AgentKind::Claude => {
            let session_id = uuid::Uuid::new_v4().to_string();
            InitialProviderPlan {
                provider_resume: Some(ProviderResume {
                    provider: agent.as_str().to_string(),
                    session_id: session_id.clone(),
                    captured_at: Zoned::now().timestamp().to_string(),
                    capture_method: "claude-explicit-session-id".to_string(),
                    resume_args: vec!["--resume".to_string(), session_id.clone()],
                    extra: BTreeMap::new(),
                }),
                launch_args: vec!["--session-id".to_string(), session_id],
                move_lineage: None,
            }
        }
        AgentKind::Codex | AgentKind::Dsh => InitialProviderPlan::default(),
    };
    Ok(plan)
}
