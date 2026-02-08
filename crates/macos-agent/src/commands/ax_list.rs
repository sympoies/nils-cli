use crate::backend::process::ProcessRunner;
use crate::backend::{AppleScriptAxBackend, AxBackendAdapter};
use crate::cli::{AxListArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{AxListRequest, AxTarget, SuccessEnvelope};
use crate::run::ActionPolicy;

pub fn run(
    format: OutputFormat,
    args: &AxListArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxListRequest {
        target: AxTarget {
            app: args.app.clone(),
            bundle_id: args.bundle_id.clone(),
        },
        role: args.role.clone(),
        title_contains: args.title_contains.clone(),
        max_depth: args.max_depth,
        limit: args.limit.map(|value| value as usize),
    };
    let backend = AppleScriptAxBackend;
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
