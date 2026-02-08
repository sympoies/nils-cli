use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxListArgs, OutputFormat};
use crate::commands::ax_common::build_target;
use crate::error::CliError;
use crate::model::{AxListRequest, SuccessEnvelope};
use crate::run::ActionPolicy;

pub fn run(
    format: OutputFormat,
    args: &AxListArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let target = build_target(
        args.session_id.clone(),
        args.app.clone(),
        args.bundle_id.clone(),
        args.window_title_contains.clone(),
    )?;

    let request = AxListRequest {
        target,
        role: args.role.clone(),
        title_contains: args.title_contains.clone(),
        identifier_contains: args.identifier_contains.clone(),
        value_contains: args.value_contains.clone(),
        subrole: args.subrole.clone(),
        focused: args.focused,
        enabled: args.enabled,
        max_depth: args.max_depth,
        limit: args.limit.map(|value| value as usize),
    };
    let backend = AutoAxBackend::default();
    let result = backend.list(runner, &request, policy.timeout_ms)?;

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.list", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            if result.nodes.is_empty() {
                println!("ax.list\tnodes=0");
            } else {
                for node in result.nodes {
                    println!(
                        "ax.list\tnode_id={}\trole={}\ttitle={}\tactions={}",
                        node.node_id,
                        node.role,
                        node.title.unwrap_or_default(),
                        node.actions.join(","),
                    );
                }
            }
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}
