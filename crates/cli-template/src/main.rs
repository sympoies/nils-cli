use clap::{Parser, Subcommand};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(
    name = "cli-template",
    version,
    about = "Template CLI for nils-cli workspace"
)]
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
    /// Render a short progress demo (progress on stderr, stdout stays clean)
    ProgressDemo,
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
        Some(Command::ProgressDemo) => {
            let progress = Progress::new(
                10,
                ProgressOptions::default()
                    .with_prefix("demo ")
                    .with_finish(ProgressFinish::Clear),
            );

            for i in 0..10_u64 {
                progress.set_message(format!("step {} of 10", i + 1));
                progress.inc(1);
                std::thread::sleep(std::time::Duration::from_millis(30));
            }

            progress.finish_and_clear();
            println!("done");
        }
        None => {
            info!("no subcommand selected");
        }
    }

    Ok(())
}
