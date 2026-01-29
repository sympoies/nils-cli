use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser)]
#[command(name = "cli-template", version, about = "Template CLI for nils-cli workspace")]
struct Cli {
    /// Log level (e.g. trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print a greeting to stdout
    Hello {
        /// Name to greet (defaults to "world")
        name: Option<String>,
    },
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_new(level)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt().with_env_filter(filter).init();
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level);

    match cli.command {
        Some(Command::Hello { name }) => {
            let name = name.unwrap_or_else(|| "world".to_string());
            let greeting = nils_common::greeting(&name);
            info!(%greeting, "generated greeting");
            println!("{greeting}");
        }
        None => {
            info!("no subcommand selected");
        }
    }

    Ok(())
}
