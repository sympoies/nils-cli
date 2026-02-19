use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use claude_cli::{agent, auth, cli, completion, config, diag};

fn main() {
    let exit_code = run();
    std::process::exit(exit_code);
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        let mut cmd = cli::Cli::command();
        if cmd.print_help().is_ok() {
            println!();
            return 0;
        }
        return 1;
    }

    if let Some(redirect_code) = handle_legacy_redirect(&args) {
        return redirect_code;
    }

    let cli = match cli::Cli::try_parse_from(std::env::args()) {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 64,
            };
            let _ = err.print();
            return code;
        }
    };

    match cli.command {
        Some(command) => match command {
            cli::Command::Agent(args) => handle_agent(&args),
            cli::Command::AuthState(args) => handle_auth_state(&args),
            cli::Command::Diag(args) => handle_diag(&args),
            cli::Command::Config(args) => handle_config(&args),
            cli::Command::Completion(args) => completion::run(args.shell),
        },
        None => {
            let mut cmd = cli::Cli::command();
            if cmd.print_help().is_ok() {
                println!();
                return 0;
            }
            1
        }
    }
}

fn handle_agent(args: &cli::AgentArgs) -> i32 {
    match &args.command {
        Some(cli::AgentCommand::Prompt { prompt }) => agent::prompt(prompt),
        Some(cli::AgentCommand::Advice { question }) => agent::advice(question),
        Some(cli::AgentCommand::Knowledge { concept }) => agent::knowledge(concept),
        None => print_subcommand_help("agent"),
    }
}

fn handle_auth_state(args: &cli::AuthStateArgs) -> i32 {
    match &args.command {
        Some(cli::AuthStateCommand::Show { output }) => auth::show(output.is_json()),
        None => print_subcommand_help("auth-state"),
    }
}

fn handle_diag(args: &cli::DiagArgs) -> i32 {
    match &args.command {
        Some(cli::DiagCommand::Healthcheck { output, timeout_ms }) => {
            diag::healthcheck(output.is_json(), *timeout_ms)
        }
        Some(cli::DiagCommand::RateLimits { output }) => {
            diag::rate_limits_unsupported(output.is_json())
        }
        None => print_subcommand_help("diag"),
    }
}

fn handle_config(args: &cli::ConfigArgs) -> i32 {
    match &args.command {
        Some(cli::ConfigCommand::Show { output }) => config::show(output.is_json()),
        Some(cli::ConfigCommand::Set { key, value }) => config::set(key, value),
        None => print_subcommand_help("config"),
    }
}

fn print_subcommand_help(name: &str) -> i32 {
    let mut cmd = cli::Cli::command();
    if let Some(subcommand) = cmd.find_subcommand_mut(name)
        && subcommand.print_help().is_ok()
    {
        println!();
        return 0;
    }
    1
}

fn handle_legacy_redirect(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    match cmd {
        "provider" | "debug" | "workflow" | "automation" => {
            eprintln!("claude-cli: use `agentctl {cmd}` for provider-neutral orchestration");
            Some(64)
        }
        "auth" => {
            eprintln!("claude-cli: use `claude-cli auth-state show`");
            Some(64)
        }
        "starship" => {
            eprintln!(
                "claude-cli: `starship` is codex-only; use `agentctl diag doctor --provider claude`"
            );
            Some(64)
        }
        "commit" => {
            eprintln!(
                "claude-cli: `agent commit` is codex-only; use `codex-cli agent commit` when needed"
            );
            Some(64)
        }
        "help" => {
            let mut command = cli::Cli::command();
            if command.print_help().is_ok() {
                println!();
                return Some(0);
            }
            Some(1)
        }
        _ => None,
    }
}
