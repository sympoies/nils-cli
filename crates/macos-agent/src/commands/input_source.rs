use crate::backend::input_source;
use crate::backend::process::ProcessRunner;
use crate::cli::{InputSourceCurrentArgs, InputSourceSwitchArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{InputSourceCurrentResult, InputSourceSwitchResult, SuccessEnvelope};
use crate::run::ActionPolicy;

pub fn run_current(
    format: OutputFormat,
    _args: &InputSourceCurrentArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let result = InputSourceCurrentResult {
        current: input_source::current(runner, policy.timeout_ms)?,
    };
    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("input-source.current", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!("input-source.current\tcurrent={}", result.current);
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

pub fn run_switch(
    format: OutputFormat,
    args: &InputSourceSwitchArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let state = input_source::switch(runner, &args.id, policy.timeout_ms)?;
    let result = InputSourceSwitchResult {
        previous: state.previous,
        current: state.current,
        switched: state.switched,
    };
    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("input-source.switch", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "input-source.switch\tprevious={}\tcurrent={}\tswitched={}",
                result.previous, result.current, result.switched
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
