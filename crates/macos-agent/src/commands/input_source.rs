use crate::backend::input_source;
use crate::backend::process::ProcessRunner;
use crate::cli::{InputSourceCurrentArgs, InputSourceSwitchArgs, OutputFormat};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{InputSourceCurrentResult, InputSourceSwitchResult};
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
            emit_json_success("input-source.current", result)?;
        }
        OutputFormat::Text => {
            println!("input-source.current\tcurrent={}", result.current);
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
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
            emit_json_success("input-source.switch", result)?;
        }
        OutputFormat::Text => {
            println!(
                "input-source.switch\tprevious={}\tcurrent={}\tswitched={}",
                result.previous, result.current, result.switched
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}
