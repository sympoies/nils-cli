use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxListArgs, OutputFormat};
use crate::commands::ax_common::build_target_from_args;
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::AxListRequest;
use crate::run::ActionPolicy;

pub fn run(
    format: OutputFormat,
    args: &AxListArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let target = build_target_from_args(&args.target)?;

    let request = AxListRequest {
        target,
        role: args.filters.role.clone(),
        title_contains: args.filters.title_contains.clone(),
        identifier_contains: args.filters.identifier_contains.clone(),
        value_contains: args.filters.value_contains.clone(),
        subrole: args.filters.subrole.clone(),
        focused: args.filters.focused,
        enabled: args.filters.enabled,
        max_depth: args.max_depth,
        limit: args.limit.map(|value| value as usize),
    };
    let backend = AutoAxBackend::default();
    let result = backend.list(runner, &request, policy.timeout_ms)?;

    match format {
        OutputFormat::Json => {
            emit_json_success("ax.list", result)?;
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
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}
