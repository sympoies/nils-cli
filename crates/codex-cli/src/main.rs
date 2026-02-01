mod cli;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use codex_cli::auth;

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
            cli::Command::Auth(args) => handle_auth(&args),
            cli::Command::Diag(args) => handle_diag(&args),
            cli::Command::Config(args) => handle_config(&args),
            cli::Command::Starship(args) => handle_starship(&args),
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
        Some(_cmd) => 0,
        None => print_subcommand_help("agent"),
    }
}

fn handle_auth(args: &cli::AuthArgs) -> i32 {
    match &args.command {
        Some(cli::AuthCommand::Use { args }) => {
            if args.len() != 1 || args[0].is_empty() {
                eprintln!("codex-use: usage: codex-use <name|name.json|email>");
                return 64;
            }
            auth::use_secret::run(&args[0]).unwrap_or(1)
        }
        Some(cli::AuthCommand::Refresh { args }) => {
            if args.len() > 1 {
                eprintln!("codex-refresh: usage: codex-refresh-auth [secret.json]");
                return 64;
            }
            auth::refresh::run(args).unwrap_or(1)
        }
        Some(cli::AuthCommand::AutoRefresh) => auth::auto_refresh::run().unwrap_or(1),
        Some(cli::AuthCommand::Current) => auth::current::run().unwrap_or(1),
        Some(cli::AuthCommand::Sync) => auth::sync::run().unwrap_or(1),
        None => print_subcommand_help("auth"),
    }
}

fn handle_diag(args: &cli::DiagArgs) -> i32 {
    match &args.command {
        Some(cli::DiagCommand::RateLimits(rate_args)) => {
            let options = codex_cli::rate_limits::RateLimitsOptions {
                clear_cache: rate_args.clear_cache,
                debug: rate_args.debug,
                cached: rate_args.cached,
                no_refresh_auth: rate_args.no_refresh_auth,
                json: rate_args.json,
                one_line: rate_args.one_line,
                all: rate_args.all,
                async_mode: rate_args.async_mode,
                jobs: rate_args.jobs.clone(),
                secret: rate_args.secret.clone(),
            };
            codex_cli::rate_limits::run(&options).unwrap_or(1)
        }
        None => print_subcommand_help("diag"),
    }
}

fn handle_config(args: &cli::ConfigArgs) -> i32 {
    match &args.command {
        Some(_cmd) => 0,
        None => print_subcommand_help("config"),
    }
}

fn handle_starship(args: &cli::StarshipArgs) -> i32 {
    let options = codex_cli::starship::StarshipOptions {
        no_5h: args.no_5h,
        ttl: args.ttl.clone(),
        time_format: args.time_format.clone(),
        refresh: args.refresh,
        is_enabled: args.is_enabled,
    };
    codex_cli::starship::run(&options)
}

fn print_subcommand_help(name: &str) -> i32 {
    let mut cmd = cli::Cli::command();
    if let Some(subcommand) = cmd.find_subcommand_mut(name) {
        if subcommand.print_help().is_ok() {
            println!();
            return 0;
        }
    }
    1
}

fn handle_legacy_redirect(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    match cmd {
        "help" => {
            let mut command = cli::Cli::command();
            if command.print_help().is_ok() {
                println!();
                return Some(0);
            }
            Some(1)
        }
        "list" => {
            eprintln!("codex-cli: use `codex-cli help`");
            Some(64)
        }
        "prompt" => {
            eprintln!("codex-cli: use `codex-cli agent prompt`");
            Some(64)
        }
        "advice" => {
            eprintln!("codex-cli: use `codex-cli agent advice`");
            Some(64)
        }
        "knowledge" => {
            eprintln!("codex-cli: use `codex-cli agent knowledge`");
            Some(64)
        }
        "commit" => {
            eprintln!("codex-cli: use `codex-cli agent commit`");
            Some(64)
        }
        "auto-refresh" => {
            eprintln!("codex-cli: use `codex-cli auth auto-refresh`");
            Some(64)
        }
        "rate-limits" => {
            eprintln!("codex-cli: use `codex-cli diag rate-limits`");
            Some(64)
        }
        _ => None,
    }
}
