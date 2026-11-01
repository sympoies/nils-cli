use clap::Parser;
use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, emit_parse_error, exit};
use nils_research_cli::{
    Engine,
    cli::{Cli, Command, completion},
    exit_code, render_text,
};

fn parse() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            use clap::error::ErrorKind;
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                error.exit();
            }
            let args: Vec<_> = std::env::args().collect();
            let json = args.iter().any(|a| a == "--format=json")
                || args.windows(2).any(|w| w == ["--format", "json"]);
            let format = if json {
                OutputFormat::Json
            } else {
                OutputFormat::Text
            };
            let code = if error.kind() == ErrorKind::InvalidSubcommand {
                "unknown-subcommand"
            } else {
                "parse-error"
            };
            // Do not echo argument values, which may contain a credential or URL query.
            std::process::exit(emit_parse_error(
                "research-cli",
                format,
                code,
                "Invalid research-cli arguments; see --help",
            ));
        }
    }
}
#[tokio::main]
async fn main() {
    let cli = parse();
    match cli.command {
        Command::Completion { shell } => print!("{}", completion(shell)),
        Command::Serve { bind } => {
            let listener = match tokio::net::TcpListener::bind(bind).await {
                Ok(listener) => listener,
                Err(_) => {
                    emit_error(
                        cli.format,
                        "bind-error",
                        "Cannot bind research HTTP listener",
                    );
                    std::process::exit(exit::RUNTIME);
                }
            };
            eprintln!(
                "research-cli HTTP listening on {}",
                listener.local_addr().expect("listener address")
            );
            if axum::serve(listener, nils_research_cli::http::router(Engine::default()))
                .await
                .is_err()
            {
                emit_error(cli.format, "serve-error", "Research HTTP server failed");
                std::process::exit(exit::RUNTIME);
            }
        }
        command => {
            let envelope = Engine::default()
                .execute(command.into_request().expect("operation"))
                .await;
            if cli.format.is_json() {
                println!(
                    "{}",
                    serde_json::to_string(&envelope).expect("envelope JSON")
                );
            } else {
                print!("{}", render_text(&envelope));
                for warning in &envelope.warnings {
                    eprintln!("warning: {warning}");
                }
                if let Some(error) = &envelope.error {
                    eprintln!("error: {}: {}", error.code, error.message);
                }
            }
            std::process::exit(exit_code(&envelope));
        }
    }
}

fn emit_error(format: OutputFormat, code: &str, message: &str) {
    if format.is_json() {
        let envelope: Envelope<()> = Envelope::failure(
            "cli.research-cli.error.v1",
            EnvelopeError::new(code, message),
        );
        println!(
            "{}",
            serde_json::to_string(&envelope).expect("error envelope JSON")
        );
    } else {
        eprintln!("error: {message}");
    }
}
