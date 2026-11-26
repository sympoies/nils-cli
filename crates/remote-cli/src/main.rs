mod transport;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint, error::ErrorKind};
use clap_complete::Shell;
use nils_common::cli_contract::{
    Envelope, EnvelopeError, OutputFormat, emit_parse_error, exit, schema_version_for,
};
use serde::Serialize;

const BINARY: &str = "remote-cli";
const EXAMPLES: &str = r#"Examples:
  remote-cli exec server --cwd '~/project' -- git status --short
  remote-cli exec server --sudo -- id -u
  remote-cli exec server -- printf '%s\n' 'a b' '$HOME' ''
  remote-cli shell server --script-file ./check.sh"#;

#[derive(Parser)]
#[command(
    name = BINARY,
    version,
    long_version = nils_build_info::long_version(env!("CARGO_PKG_VERSION")),
    about = "Run commands and scripts through OpenSSH",
    after_help = EXAMPLES
)]
struct Cli {
    /// Output format.
    #[arg(long, global = true, default_value = "text", value_enum)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Subcommands,
}

#[derive(Subcommand)]
enum Subcommands {
    /// Run a remote command with literal arguments.
    Exec {
        #[command(flatten)]
        remote: Remote,

        /// Run sudo as this user (requires --sudo).
        #[arg(long, requires = "sudo")]
        sudo_user: Option<String>,

        /// Send this local file to the remote command on stdin.
        #[arg(long, value_hint = ValueHint::FilePath)]
        stdin_file: Option<PathBuf>,

        /// Remote program and literal arguments, after --.
        #[arg(last = true, required = true, num_args = 1.., value_hint = ValueHint::CommandWithArguments)]
        argv: Vec<String>,
    },
    /// Run a local script on the remote host through stdin.
    Shell {
        #[command(flatten)]
        remote: Remote,

        /// Local script file to send on stdin.
        #[arg(long, value_hint = ValueHint::FilePath)]
        script_file: PathBuf,
    },
    /// Print a shell completion script.
    Completion {
        #[arg(value_enum)]
        shell: CompletionShell,
    },
}

#[derive(Clone, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
}

#[derive(Args)]
struct Remote {
    /// OpenSSH host alias or user@host.
    #[arg(value_parser = clap::builder::NonEmptyStringValueParser::new())]
    host: String,

    /// Remote working directory (defaults to remote home).
    #[arg(long, value_name = "DIR")]
    cwd: Option<String>,

    /// Run the command via non-interactive sudo.
    #[arg(long)]
    sudo: bool,

    /// Wall-clock timeout in seconds (0 disables the deadline).
    #[arg(long, value_name = "S", default_value_t = 40)]
    timeout: u32,

    /// OpenSSH connect timeout in seconds.
    #[arg(long, value_name = "S", default_value_t = 6, value_parser = clap::value_parser!(u32).range(..=i32::MAX as i64))]
    connect_timeout: u32,

    /// Remote shell executable (defaults to the remote login shell).
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    shell: Option<String>,
}

enum Work<'a> {
    Exec {
        argv: &'a [String],
        stdin_file: Option<&'a Path>,
        sudo_user: Option<&'a str>,
    },
    Shell {
        script_file: &'a Path,
    },
}

impl Work<'_> {
    fn name(&self) -> &'static str {
        match self {
            Self::Exec { .. } => "exec",
            Self::Shell { .. } => "shell",
        }
    }

    fn input(&self) -> Option<&Path> {
        match self {
            Self::Exec { stdin_file, .. } => *stdin_file,
            Self::Shell { script_file } => Some(script_file),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RemoteOutcome {
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Serialize)]
struct ResultData<'a> {
    host: &'a str,
    requested_cwd: Option<&'a str>,
    argv: Option<&'a [String]>,
    script_file: Option<&'a Path>,
    sudo: bool,
    sudo_user: Option<&'a str>,
    started_at: String,
    elapsed_ms: u128,
    connect_timeout_ms: u64,
    timeout_ms: u64,
    ssh_exit_code: Option<i32>,
    timed_out: bool,
    signal: Option<i32>,
    stdout: std::borrow::Cow<'a, str>,
    stderr: std::borrow::Cow<'a, str>,
    remote_outcome: RemoteOutcome,
}

#[derive(Clone, Copy)]
struct Failure {
    exit_code: i32,
    code: &'static str,
    message: &'static str,
}

impl Failure {
    const fn new(exit_code: i32, code: &'static str, message: &'static str) -> Self {
        Self {
            exit_code,
            code,
            message,
        }
    }
}

const DEADLINE: Failure = Failure::new(
    124,
    "deadline-exceeded",
    "OpenSSH exceeded the local deadline; remote outcome is unknown",
);
const SSH_TRANSPORT: Failure = Failure::new(
    255,
    "ssh-transport",
    "OpenSSH connection or session failed; remote outcome is unknown",
);

fn run(remote: Remote, work: Work<'_>, format: OutputFormat) -> io::Result<i32> {
    let schema = schema_version_for(BINARY, work.name(), 1);
    let capture = match transport::run(&remote, &work) {
        Ok(capture) => capture,
        Err(failure) => {
            if format.is_json() {
                emit_json(Envelope::<()>::failure(
                    schema,
                    EnvelopeError::new(failure.code, failure.message),
                ))?;
            } else {
                writeln!(io::stderr().lock(), "error: {}", failure.message)?;
            }
            return Ok(failure.exit_code);
        }
    };

    let (outcome, failure) = if capture.timed_out {
        (RemoteOutcome::Unknown, Some(DEADLINE))
    } else {
        match capture.status.code() {
            Some(0) => (RemoteOutcome::Succeeded, None),
            Some(255) if !capture.remote_255 => (RemoteOutcome::Unknown, Some(SSH_TRANSPORT)),
            Some(code) => (
                RemoteOutcome::Failed,
                Some(Failure::new(code, "remote-exit", "Remote command failed")),
            ),
            None => (
                RemoteOutcome::Unknown,
                Some(Failure::new(
                    capture
                        .signal()
                        .map_or(exit::RUNTIME, |signal| 128 + signal),
                    "ssh-signaled",
                    "OpenSSH was terminated by a signal; remote outcome is unknown",
                )),
            ),
        }
    };

    if format.is_json() {
        let (argv, script_file, sudo_user) = match work {
            Work::Exec {
                argv, sudo_user, ..
            } => (Some(argv), None, sudo_user),
            Work::Shell { script_file } => (None, Some(script_file), None),
        };
        let data = ResultData {
            host: &remote.host,
            requested_cwd: remote.cwd.as_deref(),
            argv,
            script_file,
            sudo: remote.sudo,
            sudo_user,
            started_at: capture.started_at.to_string(),
            elapsed_ms: capture.elapsed_ms,
            connect_timeout_ms: u64::from(remote.connect_timeout) * 1_000,
            timeout_ms: u64::from(remote.timeout) * 1_000,
            ssh_exit_code: capture.status.code(),
            timed_out: capture.timed_out,
            signal: capture.signal(),
            stdout: String::from_utf8_lossy(&capture.stdout),
            stderr: String::from_utf8_lossy(&capture.stderr),
            remote_outcome: outcome,
        };
        let mut envelope = Envelope::success(schema, data);
        if let Some(failure) = failure {
            envelope.ok = false;
            envelope.error = Some(EnvelopeError::new(failure.code, failure.message));
        }
        emit_json(envelope)?;
    } else {
        io::stdout().lock().write_all(&capture.stdout)?;
        let mut stderr = io::stderr().lock();
        stderr.write_all(&capture.stderr)?;
        if let Some(failure) = failure
            && failure.code != "remote-exit"
        {
            if !capture.stderr.is_empty() && !capture.stderr.ends_with(b"\n") {
                writeln!(stderr)?;
            }
            writeln!(stderr, "error: {}", failure.message)?;
        }
    }
    Ok(failure.map_or(exit::SUCCESS, |failure| failure.exit_code))
}

fn emit_json<T: Serialize>(envelope: Envelope<T>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &envelope)?;
    writeln!(stdout)
}

fn parse() -> Cli {
    let err = match Cli::try_parse() {
        Ok(cli) => return cli,
        Err(err) => err,
    };
    let kind = err.kind();
    if matches!(
        kind,
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        err.exit();
    }
    let mut args = std::env::args_os().skip(1);
    let mut format = OutputFormat::Text;
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if arg == "--format=json" || (arg == "--format" && args.next().is_some_and(|v| v == "json"))
        {
            format = OutputFormat::Json;
        }
    }
    let code = if kind == ErrorKind::InvalidSubcommand {
        "unknown-subcommand"
    } else {
        "parse-error"
    };
    std::process::exit(emit_parse_error(
        BINARY,
        format,
        code,
        "Invalid remote-cli arguments. See --help.",
    ));
}

fn main() {
    let cli = parse();
    let result = match cli.command {
        Subcommands::Exec {
            remote,
            argv,
            stdin_file,
            sudo_user,
        } => run(
            remote,
            Work::Exec {
                argv: &argv,
                stdin_file: stdin_file.as_deref(),
                sudo_user: sudo_user.as_deref(),
            },
            cli.format,
        ),
        Subcommands::Shell {
            remote,
            script_file,
        } => run(
            remote,
            Work::Shell {
                script_file: &script_file,
            },
            cli.format,
        ),
        Subcommands::Completion { shell } => {
            let shell = match shell {
                CompletionShell::Bash => Shell::Bash,
                CompletionShell::Zsh => Shell::Zsh,
            };
            let mut stdout = io::stdout().lock();
            let mut script = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), BINARY, &mut script);
            if shell == Shell::Bash {
                // clap_complete 4.6.12 puts the subcommand separator in
                // hyphenated binary names in case labels but not dispatch.
                let script = String::from_utf8(script).map_err(io::Error::other);
                script.and_then(|script| {
                    stdout.write_all(script.replace("__subcmd__", "__").as_bytes())
                })
            } else {
                stdout.write_all(&script)
            }
            .map(|()| exit::SUCCESS)
        }
    };
    std::process::exit(result.unwrap_or(exit::RUNTIME));
}
