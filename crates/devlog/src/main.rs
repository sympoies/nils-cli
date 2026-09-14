use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use nils_common::cli_contract::{
    Envelope, EnvelopeError, OutputFormat, emit_parse_error, exit, schema_version_for,
};
use nils_devlog::check::CheckReport;
use nils_devlog::entry::Entry;
use nils_devlog::model::{Devlog, DevlogError, EntryDate, Month};
use nils_devlog::search::SearchReport;
use serde::Serialize;

const BINARY: &str = "devlog";

#[derive(Parser)]
#[command(
    name = "devlog",
    version,
    long_version = nils_build_info::long_version(env!("CARGO_PKG_VERSION")),
    about = "Maintain and query a repository development log"
)]
struct Cli {
    /// Output format (defaults to text).
    #[arg(long, global = true, value_enum)]
    format: Option<OutputFormat>,

    /// Devlog directory. Defaults to detecting docs/devlog or docs/source/devlog.
    #[arg(long, global = true, value_name = "DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add an entry to its month file, creating the file and index link when absent.
    New {
        /// Entry title, rendered after the date in the entry heading.
        #[arg(long)]
        title: String,
        /// Entry date (defaults to today).
        #[arg(long, value_name = "YYYY-MM-DD")]
        date: Option<String>,
        /// A `Result` bullet (repeatable).
        #[arg(long = "result", value_name = "TEXT")]
        results: Vec<String>,
        /// A `Why / context` bullet (repeatable).
        #[arg(long = "why", value_name = "TEXT")]
        why: Vec<String>,
        /// An `Evidence` bullet (repeatable).
        #[arg(long = "evidence", value_name = "TEXT")]
        evidence: Vec<String>,
        /// A `Links` bullet (repeatable; the section is omitted when empty).
        #[arg(long = "link", value_name = "TEXT")]
        links: Vec<String>,
        /// A `Follow-ups` bullet (repeatable; the section is omitted when empty).
        #[arg(long = "follow-up", value_name = "TEXT")]
        follow_ups: Vec<String>,
    },
    /// Search entries for a literal, case-insensitive term.
    Search {
        /// Term to search for.
        term: String,
        /// Restrict the search to one month.
        #[arg(long, value_name = "YYYY-MM")]
        month: Option<String>,
    },
    /// Report structural problems: month filenames, index drift, entry shape, ordering.
    Check,
    /// Rewrite the README month index from the tracked month files.
    Index,
    /// Export shell completion script.
    Completion {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Serialize)]
struct NewPayload {
    month: String,
    date: String,
    path: String,
    created_month_file: bool,
    index_updated: bool,
}

#[derive(Serialize)]
struct IndexPayload {
    changed: bool,
    months: Vec<String>,
}

fn main() {
    let cli = parse_or_exit();
    let format = cli.format.unwrap_or_default();

    let exit_code = match run(&cli, format) {
        Ok(code) => code,
        Err(err) => emit_error(format, &err),
    };

    std::process::exit(exit_code);
}

fn run(cli: &Cli, format: OutputFormat) -> Result<i32, DevlogError> {
    if let Command::Completion { shell } = &cli.command {
        print_completion(*shell);
        return Ok(exit::SUCCESS);
    }

    let repo_root = repo_root()?;
    let devlog = match &cli.dir {
        Some(dir) => Devlog::at(&repo_root, dir)?,
        None => Devlog::locate(&repo_root)?,
    };

    match &cli.command {
        Command::New {
            title,
            date,
            results,
            why,
            evidence,
            links,
            follow_ups,
        } => {
            let date = match date {
                Some(value) => value.parse::<EntryDate>()?,
                None => EntryDate::today()?,
            };
            let entry = Entry {
                date: Some(date),
                title: title.clone(),
                result: results.clone(),
                why: why.clone(),
                evidence: evidence.clone(),
                links: links.clone(),
                follow_ups: follow_ups.clone(),
            };
            // Both files this command writes are checked before either is
            // touched. Letting the month file be written and the index then
            // refuse would leave a half-finished operation behind a message
            // that says nothing was written, and a retry after resolving the
            // conflict would insert the entry a second time.
            nils_devlog::index::assert_resolved(&devlog)?;
            let insertion = nils_devlog::entry::insert(&devlog, &entry, date)?;
            // A new month file is invisible until the index links it, so the
            // two mutations are one operation rather than two commands the
            // caller has to remember to pair.
            let index = nils_devlog::index::sync(&devlog)?;

            let payload = NewPayload {
                month: insertion.month.to_string(),
                date: insertion.date.to_string(),
                path: format!("{}/{}.md", devlog.relative_dir(), insertion.month),
                created_month_file: insertion.created_month_file,
                index_updated: index.changed,
            };
            emit(format, "new", 1, &payload, |payload| {
                println!("added {} to {}", payload.date, payload.path);
                if payload.created_month_file {
                    println!("created {}", payload.path);
                }
                if payload.index_updated {
                    println!("updated {}/README.md", devlog.relative_dir());
                }
            });
            Ok(exit::SUCCESS)
        }
        Command::Search { term, month } => {
            let month = month.as_deref().map(str::parse::<Month>).transpose()?;
            let report = nils_devlog::search::search(&devlog, term, month)?;
            let found = !report.matches.is_empty();
            let render = |report: &SearchReport| {
                for entry in &report.matches {
                    println!("{}.md:{}:{}", entry.month, entry.line_number, entry.line);
                }
                if report.matches.is_empty() {
                    eprintln!("(no matches for '{}')", report.term);
                }
            };
            if found {
                emit(format, "search", 1, &report, render);
                Ok(exit::SUCCESS)
            } else {
                emit_failure(
                    format,
                    "search",
                    1,
                    &report,
                    "no-matches",
                    "no devlog entry matched the search term",
                    render,
                );
                Ok(exit::RUNTIME)
            }
        }
        Command::Check => {
            let report = nils_devlog::check::check(&devlog)?;
            if report.ok() {
                emit(format, "check", 1, &report, print_check);
                Ok(exit::SUCCESS)
            } else {
                emit_failure(
                    format,
                    "check",
                    1,
                    &report,
                    "structural-problems",
                    "the development log has structural problems",
                    print_check,
                );
                Ok(exit::DATA)
            }
        }
        Command::Index => {
            let update = nils_devlog::index::sync(&devlog)?;
            let payload = IndexPayload {
                changed: update.changed,
                months: update.months.iter().map(Month::to_string).collect(),
            };
            emit(format, "index", 1, &payload, |payload| {
                if payload.changed {
                    println!("index updated: {} months", payload.months.len());
                } else {
                    println!("index already current: {} months", payload.months.len());
                }
            });
            Ok(exit::SUCCESS)
        }
        Command::Completion { .. } => unreachable!("handled before devlog resolution"),
    }
}

fn print_check(report: &CheckReport) {
    println!(
        "{}: {} months, {} entries",
        report.devlog_dir, report.month_count, report.entry_count
    );
    if report.problems.is_empty() {
        println!("ok: no structural problems");
        return;
    }
    for problem in &report.problems {
        println!("{}: {} - {}", problem.kind, problem.path, problem.detail);
    }
    println!("{} problem(s)", report.problems.len());
}

fn emit<T, F>(format: OutputFormat, command: &str, version: u32, payload: &T, render_text: F)
where
    T: Serialize,
    F: FnOnce(&T),
{
    emit_outcome(format, command, version, payload, None, render_text);
}

/// Render a payload whose command outcome is a failure.
///
/// `docs/specs/cli-output-contract-v1.md` requires `ok` to mirror success or
/// failure, so a `check` that found problems, or a `search` that matched
/// nothing, must not report `ok: true` beside its non-zero exit. The payload
/// still travels, under `error.details`, because it is the useful part of the
/// answer rather than a diagnostic about one.
fn emit_failure<T, F>(
    format: OutputFormat,
    command: &str,
    version: u32,
    payload: &T,
    code: &str,
    message: &str,
    render_text: F,
) where
    T: Serialize,
    F: FnOnce(&T),
{
    emit_outcome(
        format,
        command,
        version,
        payload,
        Some((code, message)),
        render_text,
    );
}

fn emit_outcome<T, F>(
    format: OutputFormat,
    command: &str,
    version: u32,
    payload: &T,
    failure: Option<(&str, &str)>,
    render_text: F,
) where
    T: Serialize,
    F: FnOnce(&T),
{
    match format {
        OutputFormat::Json => {
            let schema_version = schema_version_for(BINARY, command, version);
            let serialized = match failure {
                None => serde_json::to_string(&Envelope::success(schema_version, payload)),
                Some((code, message)) => {
                    let details = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
                    let error = EnvelopeError::new(code, message).with_details(details);
                    serde_json::to_string(&Envelope::<()>::failure(schema_version, error))
                }
            };
            match serialized {
                Ok(serialized) => println!("{serialized}"),
                Err(err) => eprintln!("error: failed to serialize envelope: {err}"),
            }
        }
        OutputFormat::Text => render_text(payload),
    }
}

fn emit_error(format: OutputFormat, err: &DevlogError) -> i32 {
    let code = exit_code_for(err);
    match format {
        OutputFormat::Json => {
            let envelope = Envelope::<()>::failure(
                schema_version_for(BINARY, "error", 1),
                EnvelopeError::new(err.code(), err.to_string()),
            );
            match serde_json::to_string(&envelope) {
                Ok(serialized) => println!("{serialized}"),
                Err(_) => return exit::SOFTWARE,
            }
        }
        OutputFormat::Text => eprintln!("error: {err}"),
    }
    code
}

fn exit_code_for(err: &DevlogError) -> i32 {
    match err {
        DevlogError::InvalidMonth { .. } | DevlogError::InvalidDate { .. } => exit::USAGE,
        DevlogError::NotFound { .. }
        | DevlogError::NotADirectory { .. }
        | DevlogError::NotAGitWorkTree => exit::UNAVAILABLE,
        DevlogError::MissingMonthFile { .. } | DevlogError::MissingHeading { .. } => exit::RUNTIME,
        // The file on disk is unusable input, which is the same class `check`
        // reports its structural problems under.
        DevlogError::ConflictMarkers { .. } => exit::DATA,
        DevlogError::Io { .. } => exit::SOFTWARE,
    }
}

/// Write the completion script for `shell` to stdout.
///
/// Bash output is normalized before it is emitted: `clap_complete` names its
/// per-subcommand cases `<bin>__subcmd__<name>`, while this workspace's
/// completion assets and `scripts/ci/completion-flag-parity-audit.sh` expect
/// `<bin>__<name>`. Every other crate here applies the same replacement, so
/// skipping it produces a script with no per-subcommand cases at all.
fn print_completion(shell: clap_complete::Shell) {
    let mut command = Cli::command();
    if matches!(shell, clap_complete::Shell::Bash) {
        let mut rendered = Vec::new();
        clap_complete::generate(shell, &mut command, BINARY, &mut rendered);
        let normalized = String::from_utf8(rendered)
            .expect("bash completion is valid UTF-8")
            .replace("__subcmd__", "__");
        if let Err(err) = std::io::Write::write_all(&mut std::io::stdout(), normalized.as_bytes()) {
            eprintln!("error: failed to write bash completion: {err}");
        }
        return;
    }
    clap_complete::generate(shell, &mut command, BINARY, &mut std::io::stdout());
}

fn repo_root() -> Result<PathBuf, DevlogError> {
    nils_common::git::repo_root()
        .ok()
        .flatten()
        .ok_or(DevlogError::NotAGitWorkTree)
}

fn detect_format_from_argv() -> OutputFormat {
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--format"
            && let Some(next) = iter.next()
            && next.eq_ignore_ascii_case("json")
        {
            return OutputFormat::Json;
        }
        if let Some(rest) = arg.strip_prefix("--format=")
            && rest.eq_ignore_ascii_case("json")
        {
            return OutputFormat::Json;
        }
    }
    OutputFormat::Text
}

fn render_clap_message(err: &clap::Error) -> String {
    err.to_string()
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| {
            let line = line.trim();
            line.strip_prefix("error:")
                .map(str::trim)
                .unwrap_or(line)
                .to_string()
        })
        .unwrap_or_else(|| "command-line parse failed".to_string())
}

fn parse_or_exit() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            use clap::error::ErrorKind;
            let kind = err.kind();
            if matches!(
                kind,
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                err.exit();
            }
            let format = detect_format_from_argv();
            let code = match kind {
                ErrorKind::InvalidSubcommand => "unknown-subcommand",
                _ => "parse-error",
            };
            let message = render_clap_message(&err);
            std::process::exit(emit_parse_error(BINARY, format, code, &message));
        }
    }
}
