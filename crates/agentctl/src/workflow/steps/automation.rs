use super::{fzf_cli, image_processing, macos_agent, screen_record};
use crate::workflow::schema::{AutomationStep, AutomationTool};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AutomationCommandProvenance {
    pub tool: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AutomationInvocation {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub provenance: AutomationCommandProvenance,
}

impl AutomationInvocation {
    pub fn new(
        tool: AutomationTool,
        command: impl Into<String>,
        args: Vec<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        let command = command.into();
        let provenance = AutomationCommandProvenance {
            tool: tool.as_id().to_string(),
            command: command.clone(),
            args: args.clone(),
        };

        Self {
            command,
            args,
            env,
            provenance,
        }
    }
}

pub fn resolve_automation_invocation(step: &AutomationStep) -> AutomationInvocation {
    match step.tool {
        AutomationTool::MacosAgent => macos_agent::build(step),
        AutomationTool::ScreenRecord => screen_record::build(step),
        AutomationTool::ImageProcessing => image_processing::build(step),
        AutomationTool::FzfCli => fzf_cli::build(step),
    }
}
