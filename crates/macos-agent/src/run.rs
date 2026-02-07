use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::backend::process::RealProcessRunner;
use crate::cli::{
    AppsCommand, Cli, CommandGroup, InputCommand, ObserveCommand, OutputFormat, PreflightArgs,
    ProfileCommand, ScenarioCommand, WaitCommand, WindowCommand, WindowsCommand,
};
use crate::commands;
use crate::error::CliError;
use crate::model::{ActionMeta, ActionPolicyResult};
use crate::preflight;
use crate::retry::RetryPolicy;
use crate::test_mode;

static ACTION_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
pub struct ActionPolicy {
    pub dry_run: bool,
    pub retries: u8,
    pub retry_delay_ms: u64,
    pub timeout_ms: u64,
}

impl ActionPolicy {
    pub fn retry_policy(self) -> RetryPolicy {
        RetryPolicy {
            retries: self.retries,
            retry_delay_ms: self.retry_delay_ms,
        }
    }
}

pub fn run(cli: Cli) -> Result<(), CliError> {
    ensure_supported_platform()?;
    validate_format_support(&cli)?;

    let policy = ActionPolicy {
        dry_run: cli.dry_run,
        retries: cli.retries,
        retry_delay_ms: cli.retry_delay_ms,
        timeout_ms: cli.timeout_ms,
    };
    let runner = RealProcessRunner;

    match cli.command {
        CommandGroup::Preflight(args) => run_preflight(cli.format, args),
        CommandGroup::Windows {
            command: WindowsCommand::List(args),
        } => commands::list::run_windows_list(cli.format, &args),
        CommandGroup::Apps {
            command: AppsCommand::List(args),
        } => commands::list::run_apps_list(cli.format, &args),
        CommandGroup::Observe {
            command: ObserveCommand::Screenshot(args),
        } => commands::observe::run_screenshot(cli.format, &args),
        CommandGroup::Wait {
            command: WaitCommand::Sleep(args),
        } => commands::wait::run_sleep(cli.format, &args),
        CommandGroup::Wait {
            command: WaitCommand::AppActive(args),
        } => commands::wait::run_app_active(cli.format, &args),
        CommandGroup::Wait {
            command: WaitCommand::WindowPresent(args),
        } => commands::wait::run_window_present(cli.format, &args),
        CommandGroup::Scenario {
            command: ScenarioCommand::Run(args),
        } => commands::scenario::run(cli.format, &args),
        CommandGroup::Profile {
            command: ProfileCommand::Validate(args),
        } => commands::profile::run_validate(cli.format, &args),
        CommandGroup::Profile {
            command: ProfileCommand::Init(args),
        } => commands::profile::run_init(cli.format, &args),
        CommandGroup::Window {
            command: WindowCommand::Activate(args),
        } => commands::window_activate::run(cli.format, &args, policy, &runner),
        CommandGroup::Input {
            command: InputCommand::Click(args),
        } => commands::input_click::run(cli.format, &args, policy, &runner),
        CommandGroup::Input {
            command: InputCommand::Type(args),
        } => commands::input_type::run(cli.format, &args, policy, &runner),
        CommandGroup::Input {
            command: InputCommand::Hotkey(args),
        } => commands::input_hotkey::run(cli.format, &args, policy, &runner),
    }
}

fn ensure_supported_platform() -> Result<(), CliError> {
    if cfg!(target_os = "macos") || test_mode::enabled() {
        Ok(())
    } else {
        Err(CliError::unsupported_platform())
    }
}

fn validate_format_support(cli: &Cli) -> Result<(), CliError> {
    if cli.format != OutputFormat::Tsv {
        return Ok(());
    }

    let tsv_allowed = matches!(
        &cli.command,
        CommandGroup::Windows {
            command: WindowsCommand::List(_),
        } | CommandGroup::Apps {
            command: AppsCommand::List(_),
        }
    );

    if tsv_allowed {
        Ok(())
    } else {
        Err(CliError::usage(
            "--format tsv is only supported for `windows list` and `apps list`",
        ))
    }
}

fn run_preflight(format: OutputFormat, args: PreflightArgs) -> Result<(), CliError> {
    let snapshot = preflight::collect_system_snapshot();
    let probes = if args.include_probes {
        preflight::run_live_probes()
    } else {
        Vec::new()
    };
    let report = preflight::build_report_with_probes(snapshot, args.strict, probes);

    match format {
        OutputFormat::Text => println!("{}", preflight::render_text(&report)),
        OutputFormat::Json => println!("{}", preflight::render_json(&report)),
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

pub fn next_action_id(command: &str) -> String {
    let sequence = ACTION_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{command}-{}-{sequence}", test_mode::timestamp_token())
}

pub fn build_action_meta(action_id: String, started: Instant, policy: ActionPolicy) -> ActionMeta {
    build_action_meta_with_attempts(
        action_id,
        started,
        policy,
        if policy.dry_run { 0 } else { 1 },
    )
}

pub fn build_action_meta_with_attempts(
    action_id: String,
    started: Instant,
    policy: ActionPolicy,
    attempts_used: u8,
) -> ActionMeta {
    ActionMeta {
        action_id,
        elapsed_ms: started.elapsed().as_millis() as u64,
        dry_run: policy.dry_run,
        retries: policy.retries,
        attempts_used,
        timeout_ms: policy.timeout_ms,
    }
}

pub fn action_policy_result(policy: ActionPolicy) -> ActionPolicyResult {
    ActionPolicyResult {
        dry_run: policy.dry_run,
        retries: policy.retries,
        retry_delay_ms: policy.retry_delay_ms,
        timeout_ms: policy.timeout_ms,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use crate::cli::{Cli, OutputFormat};

    use super::{ensure_supported_platform, validate_format_support};

    #[test]
    fn rejects_tsv_for_non_list_commands() {
        let cli = Cli::try_parse_from(["macos-agent", "--format", "tsv", "preflight"])
            .expect("args should parse");
        let err = validate_format_support(&cli).expect_err("tsv should fail for preflight");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("windows list"));
    }

    #[test]
    fn allows_tsv_for_windows_list() {
        let cli = Cli::try_parse_from(["macos-agent", "--format", "tsv", "windows", "list"])
            .expect("args should parse");
        assert_eq!(cli.format, OutputFormat::Tsv);
        validate_format_support(&cli).expect("tsv should be accepted for windows list");
    }

    #[test]
    fn platform_gate_maps_non_macos_to_usage_error_unless_test_mode() {
        let result = ensure_supported_platform();
        #[cfg(target_os = "macos")]
        assert!(result.is_ok());

        #[cfg(not(target_os = "macos"))]
        {
            let err = result.expect_err("non-macos should be rejected by default");
            assert_eq!(err.exit_code(), 2);
            assert!(err.to_string().contains("macOS"));
        }
    }
}
