use super::automation::AutomationInvocation;
use crate::workflow::schema::{AutomationStep, AutomationTool};

pub fn build(step: &AutomationStep) -> AutomationInvocation {
    let args = if step.args.is_empty() {
        vec!["info".to_string(), "--help".to_string()]
    } else {
        step.args.clone()
    };

    AutomationInvocation::new(
        AutomationTool::ImageProcessing,
        "image-processing",
        args,
        Vec::new(),
    )
}
