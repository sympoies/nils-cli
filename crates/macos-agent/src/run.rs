use crate::cli::{
    AppsCommand, Cli, CommandGroup, InputCommand, ObserveCommand, OutputFormat, PreflightArgs,
    WindowCommand, WindowsCommand,
};
use crate::error::CliError;
use crate::preflight;

pub fn run(cli: Cli) -> Result<(), CliError> {
    ensure_supported_platform()?;
    validate_format_support(&cli)?;

    match cli.command {
        CommandGroup::Preflight(args) => run_preflight(cli.format, args),
        CommandGroup::Windows {
            command: WindowsCommand::List(_),
        } => not_implemented("windows.list"),
        CommandGroup::Apps {
            command: AppsCommand::List(_),
        } => not_implemented("apps.list"),
        CommandGroup::Window {
            command: WindowCommand::Activate(_),
        } => not_implemented("window.activate"),
        CommandGroup::Input {
            command: InputCommand::Click(_),
        } => not_implemented("input.click"),
        CommandGroup::Input {
            command: InputCommand::Type(_),
        } => not_implemented("input.type"),
        CommandGroup::Input {
            command: InputCommand::Hotkey(_),
        } => not_implemented("input.hotkey"),
        CommandGroup::Observe {
            command: ObserveCommand::Screenshot(_),
        } => not_implemented("observe.screenshot"),
    }
}

fn ensure_supported_platform() -> Result<(), CliError> {
    if cfg!(target_os = "macos") {
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
    let report = preflight::build_report(snapshot, args.strict);

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

fn not_implemented(command: &str) -> Result<(), CliError> {
    Err(CliError::runtime(format!(
        "{command} is not implemented yet; this will be added in a follow-up task"
    )))
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
    fn platform_gate_maps_non_macos_to_usage_error() {
        let result = ensure_supported_platform();
        #[cfg(target_os = "macos")]
        assert!(result.is_ok());

        #[cfg(not(target_os = "macos"))]
        {
            let err = result.expect_err("non-macos should be rejected");
            assert_eq!(err.exit_code(), 2);
            assert!(err.to_string().contains("macOS"));
        }
    }
}
