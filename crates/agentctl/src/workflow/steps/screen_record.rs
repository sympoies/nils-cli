use super::automation::AutomationInvocation;
use crate::workflow::schema::{AutomationStep, AutomationTool};

pub fn build(step: &AutomationStep) -> AutomationInvocation {
    let args = if step.args.is_empty() {
        vec!["--preflight".to_string()]
    } else {
        step.args.clone()
    };

    AutomationInvocation::new(
        AutomationTool::ScreenRecord,
        "screen-record",
        args,
        Vec::new(),
    )
}
