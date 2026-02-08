use super::automation::AutomationInvocation;
use crate::workflow::schema::{AutomationStep, AutomationTool};

pub fn build(step: &AutomationStep) -> AutomationInvocation {
    let args = if step.args.is_empty() {
        vec![
            "--format".to_string(),
            "json".to_string(),
            "preflight".to_string(),
        ]
    } else {
        step.args.clone()
    };

    AutomationInvocation::new(AutomationTool::MacosAgent, "macos-agent", args, Vec::new())
}
