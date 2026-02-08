use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxActionPerformArgs, OutputFormat};
use crate::commands::ax_common::{build_selector, build_target, AxSelectorInput};
use crate::error::CliError;
use crate::model::{AxActionPerformRequest, AxActionPerformResult, SuccessEnvelope};
use crate::run::ActionPolicy;

pub fn run_perform(
    format: OutputFormat,
    args: &AxActionPerformArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxActionPerformRequest {
        target: build_target(
            args.session_id.clone(),
            args.app.clone(),
            args.bundle_id.clone(),
            args.window_title_contains.clone(),
        )?,
        selector: build_selector(AxSelectorInput {
            node_id: args.node_id.clone(),
            role: args.role.clone(),
            title_contains: args.title_contains.clone(),
            identifier_contains: args.identifier_contains.clone(),
            value_contains: args.value_contains.clone(),
            subrole: args.subrole.clone(),
            focused: args.focused,
            enabled: args.enabled,
            nth: args.nth,
        })?,
        name: args.name.clone(),
    };

    let result = if policy.dry_run {
        AxActionPerformResult {
            node_id: request.selector.node_id.clone(),
            matched_count: 0,
            name: request.name.clone(),
            performed: false,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.action_perform(runner, &request, policy.timeout_ms)?
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.action.perform", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.action.perform\tnode_id={}\tname={}\tmatched_count={}\tperformed={}",
                result.node_id.unwrap_or_default(),
                result.name,
                result.matched_count,
                result.performed
            );
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}
