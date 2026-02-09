use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use clap::{Parser, error::ErrorKind};
use macos_agent::cli::{Cli, ErrorFormat};
use macos_agent::error::CliError;
use macos_agent::model::ErrorEnvelope;
use macos_agent::run::{command_label, run};
use macos_agent::test_mode;

static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn main() -> ExitCode {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let requested_error_format = detect_error_format(&raw_args);
    let requested_trace = detect_trace_request(&raw_args);
    let started = Instant::now();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let is_info = matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let code = if is_info { 0 } else { err.exit_code() };
            if is_info || requested_error_format == ErrorFormat::Text {
                let _ = err.print();
            } else {
                let mut parse_error = CliError::usage(err.to_string())
                    .with_operation("cli.parse")
                    .with_hint("Run `macos-agent --help` to inspect supported command syntax.");
                if requested_trace
                    && let Err(trace_error) = write_trace(
                        None,
                        &raw_args,
                        started.elapsed().as_millis() as u64,
                        false,
                        Some(&parse_error),
                    )
                {
                    parse_error = parse_error.with_hint(trace_write_hint(&trace_error));
                }
                emit_error(&parse_error, requested_error_format);
            }
            return ExitCode::from(code as u8);
        }
    };

    let trace_enabled = cli.trace || cli.trace_dir.is_some();
    if trace_enabled && let Err(error) = ensure_trace_dir_writable(&cli) {
        emit_error(&error, cli.error_format);
        return ExitCode::from(error.exit_code());
    }

    match run(cli.clone()) {
        Ok(()) => {
            if trace_enabled {
                let _ = write_trace(
                    Some(&cli),
                    &raw_args,
                    started.elapsed().as_millis() as u64,
                    true,
                    None,
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            let mut err = err;
            if trace_enabled
                && let Err(trace_error) = write_trace(
                    Some(&cli),
                    &raw_args,
                    started.elapsed().as_millis() as u64,
                    false,
                    Some(&err),
                )
            {
                err = err.with_hint(trace_write_hint(&trace_error));
            }
            emit_error(&err, cli.error_format);
            ExitCode::from(err.exit_code())
        }
    }
}

fn ensure_trace_dir_writable(cli: &Cli) -> Result<(), CliError> {
    let dir = trace_dir(Some(cli));
    std::fs::create_dir_all(&dir).map_err(|error| trace_write_error(&dir, &error))?;
    let probe = dir.join(format!(
        ".trace-write-probe-{}-{}",
        std::process::id(),
        TRACE_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&probe, b"").map_err(|error| trace_write_error(&dir, &error))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

fn emit_error(err: &CliError, format: ErrorFormat) {
    match format {
        ErrorFormat::Text => eprintln!("{err}"),
        ErrorFormat::Json => {
            let payload = ErrorEnvelope::from_error(err);
            if let Ok(raw) = serde_json::to_string(&payload) {
                eprintln!("{raw}");
            } else {
                eprintln!("{err}");
            }
        }
    }
}

fn detect_error_format(args: &[String]) -> ErrorFormat {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--error-format" {
            if let Some(value) = iter.next()
                && value.eq_ignore_ascii_case("json")
            {
                return ErrorFormat::Json;
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--error-format=")
            && value.eq_ignore_ascii_case("json")
        {
            return ErrorFormat::Json;
        }
    }
    ErrorFormat::Text
}

fn detect_trace_request(args: &[String]) -> bool {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--trace" {
            return true;
        }
        if arg == "--trace-dir" {
            return iter.next().is_some();
        }
        if arg.starts_with("--trace-dir=") {
            return true;
        }
    }
    false
}

fn write_trace(
    cli: Option<&Cli>,
    raw_args: &[String],
    elapsed_ms: u64,
    ok: bool,
    err: Option<&CliError>,
) -> Result<(), std::io::Error> {
    let dir = trace_dir(cli);
    std::fs::create_dir_all(&dir)?;
    let sequence = TRACE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let filename = format!(
        "trace-{}-{}-{sequence}.json",
        test_mode::timestamp_token(),
        std::process::id()
    );
    let path = dir.join(filename);
    let payload = serde_json::json!({
        "schema_version": 1,
        "ok": ok,
        "elapsed_ms": elapsed_ms,
        "command": cli.map(command_label).unwrap_or("<parse-error>").to_string(),
        "args": raw_args,
        "error": err.map(|value| ErrorEnvelope::from_error(value).error),
        "policy": cli.map(|value| serde_json::json!({
            "dry_run": value.dry_run,
            "retries": value.retries,
            "retry_delay_ms": value.retry_delay_ms,
            "timeout_ms": value.timeout_ms
        })),
    });
    let body = serde_json::to_vec_pretty(&payload)
        .map_err(|ser_err| std::io::Error::other(ser_err.to_string()))?;
    std::fs::write(path, body)
}

fn trace_write_error(dir: &Path, error: &std::io::Error) -> CliError {
    CliError::runtime(format!(
        "trace output directory is not writable: {} ({error})",
        dir.display()
    ))
    .with_operation("trace.write")
    .with_hint("Use --trace-dir with a writable directory path.")
}

fn trace_write_hint(error: &std::io::Error) -> String {
    format!("Trace write failed: {error}. Use --trace-dir with a writable directory path.")
}

fn trace_dir(cli: Option<&Cli>) -> PathBuf {
    if let Some(cli) = cli
        && let Some(path) = cli.trace_dir.clone()
    {
        return path;
    }
    codex_out_dir().join("macos-agent-trace")
}

fn codex_out_dir() -> PathBuf {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("out");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".codex").join("out");
    }
    PathBuf::from(".codex").join("out")
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use macos_agent::cli::Cli;
    use macos_agent::run::command_label;

    #[test]
    fn command_label_maps_ax_subcommands() {
        let list = Cli::try_parse_from(["macos-agent", "ax", "list"]).expect("ax list parse");
        assert_eq!(command_label(&list), "ax.list");

        let click = Cli::try_parse_from(["macos-agent", "ax", "click", "--node-id", "node-1"])
            .expect("ax click parse");
        assert_eq!(command_label(&click), "ax.click");

        let typ = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "type",
            "--node-id",
            "node-1",
            "--text",
            "hello",
        ])
        .expect("ax type parse");
        assert_eq!(command_label(&typ), "ax.type");

        let attr_get = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "attr",
            "get",
            "--node-id",
            "1.1",
            "--name",
            "AXRole",
        ])
        .expect("ax attr get parse");
        assert_eq!(command_label(&attr_get), "ax.attr.get");

        let action = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "action",
            "perform",
            "--node-id",
            "1.1",
            "--name",
            "AXPress",
        ])
        .expect("ax action perform parse");
        assert_eq!(command_label(&action), "ax.action.perform");

        let session_start = Cli::try_parse_from(["macos-agent", "ax", "session", "start"])
            .expect("ax session start parse");
        assert_eq!(command_label(&session_start), "ax.session.start");

        let watch_poll =
            Cli::try_parse_from(["macos-agent", "ax", "watch", "poll", "--watch-id", "axw-1"])
                .expect("ax watch poll parse");
        assert_eq!(command_label(&watch_poll), "ax.watch.poll");

        let current = Cli::try_parse_from(["macos-agent", "input-source", "current"])
            .expect("input-source current parse");
        assert_eq!(command_label(&current), "input-source.current");

        let switch = Cli::try_parse_from(["macos-agent", "input-source", "switch", "--id", "abc"])
            .expect("input-source switch parse");
        assert_eq!(command_label(&switch), "input-source.switch");
    }
}
