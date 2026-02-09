use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const WORKFLOW_SCHEMA_VERSION: &str = "agentctl.workflow.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, alias = "mode")]
    pub on_error: WorkflowOnError,
    pub steps: Vec<WorkflowStep>,
}

impl WorkflowDocument {
    pub fn validate(&self) -> Result<(), WorkflowSchemaError> {
        if self.schema_version != WORKFLOW_SCHEMA_VERSION {
            return Err(WorkflowSchemaError::new(format!(
                "unsupported schema_version '{}'; expected '{}'",
                self.schema_version, WORKFLOW_SCHEMA_VERSION
            )));
        }

        if self.steps.is_empty() {
            return Err(WorkflowSchemaError::new(
                "workflow must define at least one step",
            ));
        }

        let mut ids = BTreeSet::new();
        for step in &self.steps {
            let step_id = step.id().trim();
            if step_id.is_empty() {
                return Err(WorkflowSchemaError::new(
                    "workflow step id must not be empty",
                ));
            }

            if !ids.insert(step_id.to_string()) {
                return Err(WorkflowSchemaError::new(format!(
                    "duplicate workflow step id '{}'",
                    step_id
                )));
            }

            if step.retry().max_attempts == 0 {
                return Err(WorkflowSchemaError::new(format!(
                    "step '{}' retry.max_attempts must be >= 1",
                    step_id
                )));
            }

            if step.timeout_ms() == Some(0) {
                return Err(WorkflowSchemaError::new(format!(
                    "step '{}' timeout_ms must be >= 1 when provided",
                    step_id
                )));
            }

            if let WorkflowStep::Provider(provider) = step
                && provider.task.trim().is_empty()
            {
                return Err(WorkflowSchemaError::new(format!(
                    "provider step '{}' task must not be empty",
                    step_id
                )));
            }
        }

        Ok(())
    }
}

fn default_schema_version() -> String {
    WORKFLOW_SCHEMA_VERSION.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowOnError {
    #[default]
    FailFast,
    ContinueOnError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WorkflowStep {
    Provider(ProviderStep),
    Automation(AutomationStep),
}

impl WorkflowStep {
    pub fn id(&self) -> &str {
        match self {
            Self::Provider(step) => &step.id,
            Self::Automation(step) => &step.id,
        }
    }

    pub fn retry(&self) -> RetryPolicy {
        match self {
            Self::Provider(step) => step.retry,
            Self::Automation(step) => step.retry,
        }
    }

    pub fn timeout_ms(&self) -> Option<u64> {
        match self {
            Self::Provider(step) => step.timeout_ms,
            Self::Automation(step) => step.timeout_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStep {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub retry: RetryPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationStep {
    pub id: String,
    pub tool: AutomationTool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub retry: RetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutomationTool {
    MacosAgent,
    ScreenRecord,
    ImageProcessing,
    FzfCli,
}

impl AutomationTool {
    pub const fn as_id(self) -> &'static str {
        match self {
            Self::MacosAgent => "macos-agent",
            Self::ScreenRecord => "screen-record",
            Self::ImageProcessing => "image-processing",
            Self::FzfCli => "fzf-cli",
        }
    }

    pub const fn command(self) -> &'static str {
        self.as_id()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff_ms: u64,
}

impl RetryPolicy {
    pub fn normalized_max_attempts(self) -> u32 {
        self.max_attempts.max(1)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            backoff_ms: 0,
        }
    }
}

fn default_max_attempts() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSchemaError {
    message: String,
}

impl WorkflowSchemaError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkflowSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for WorkflowSchemaError {}
