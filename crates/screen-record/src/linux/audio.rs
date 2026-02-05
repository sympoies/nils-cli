use std::process::Command;

use nils_common::process::find_in_path;

use crate::cli::AudioMode;
use crate::error::CliError;

const OUTPUT_LIMIT_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPulseInputs {
    /// PulseAudio-compatible source names in input order (system first, then mic).
    pub(crate) sources: Vec<String>,
}

pub(crate) fn resolve_pulse_inputs(mode: AudioMode) -> Result<ResolvedPulseInputs, CliError> {
    match mode {
        AudioMode::Off => Ok(ResolvedPulseInputs {
            sources: Vec::new(),
        }),
        AudioMode::Mic => Ok(ResolvedPulseInputs {
            sources: vec![default_source()?],
        }),
        AudioMode::System => Ok(ResolvedPulseInputs {
            sources: vec![default_sink_monitor_source()?],
        }),
        AudioMode::Both => Ok(ResolvedPulseInputs {
            sources: vec![default_sink_monitor_source()?, default_source()?],
        }),
    }
}

fn default_sink_monitor_source() -> Result<String, CliError> {
    let sink = default_sink()?;
    let monitor = format!("{sink}.monitor");
    if !pulse_source_exists(&monitor)? {
        return Err(CliError::runtime(format!(
            "failed to resolve system audio source: {monitor} not found (install pipewire-pulse or pulseaudio-utils)"
        )));
    }
    Ok(monitor)
}

fn default_sink() -> Result<String, CliError> {
    ensure_pactl()?;
    if let Some(value) = pactl_stdout_trimmed(&["get-default-sink"])? {
        return Ok(value);
    }
    let info = pactl_stdout_all(&["info"])?;
    parse_info_field(&info, "Default Sink:")
        .ok_or_else(|| CliError::runtime("failed to resolve default sink via pactl"))
}

fn default_source() -> Result<String, CliError> {
    ensure_pactl()?;
    if let Some(value) = pactl_stdout_trimmed(&["get-default-source"])? {
        return Ok(value);
    }
    let info = pactl_stdout_all(&["info"])?;
    parse_info_field(&info, "Default Source:")
        .ok_or_else(|| CliError::runtime("failed to resolve default source via pactl"))
}

fn pulse_source_exists(name: &str) -> Result<bool, CliError> {
    ensure_pactl()?;

    if let Ok(Some(list)) = pactl_stdout_trimmed(&["list", "short", "sources"]) {
        for line in list.lines() {
            let mut parts = line.split('\t');
            let _idx = parts.next();
            let source_name = parts.next().unwrap_or_default().trim();
            if source_name == name {
                return Ok(true);
            }
        }
        return Ok(false);
    }

    let list = pactl_stdout_all(&["list", "sources"])?;
    for line in list.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            if rest.trim() == name {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn parse_info_field(info: &str, field: &str) -> Option<String> {
    for line in info.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(field) {
            let value = rest.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn ensure_pactl() -> Result<(), CliError> {
    if find_in_path("pactl").is_none() {
        return Err(CliError::runtime(
            "pactl not found on PATH (install pipewire-pulse or pulseaudio-utils)",
        ));
    }
    Ok(())
}

fn pactl_stdout_trimmed(args: &[&str]) -> Result<Option<String>, CliError> {
    let out = pactl_run(args)?;
    if !out.status.success() {
        return Ok(None);
    }
    let value = out.stdout.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.to_string()))
    }
}

fn pactl_stdout_all(args: &[&str]) -> Result<String, CliError> {
    let out = pactl_run(args)?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    Err(CliError::runtime(format!(
        "pactl {} failed{}{}",
        args.join(" "),
        exit_code_suffix(out.status.code()),
        output_snippet(&out.stderr)
    )))
}

struct PactlOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn pactl_run(args: &[&str]) -> Result<PactlOutput, CliError> {
    let output = Command::new("pactl")
        .args(args)
        .output()
        .map_err(map_spawn_error)?;
    Ok(PactlOutput {
        status: output.status,
        stdout: bounded_utf8(&output.stdout),
        stderr: bounded_utf8(&output.stderr),
    })
}

fn bounded_utf8(bytes: &[u8]) -> String {
    let slice = if bytes.len() > OUTPUT_LIMIT_BYTES {
        &bytes[bytes.len() - OUTPUT_LIMIT_BYTES..]
    } else {
        bytes
    };
    String::from_utf8_lossy(slice).to_string()
}

fn output_snippet(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(": {trimmed}")
    }
}

fn exit_code_suffix(code: Option<i32>) -> String {
    match code {
        Some(code) => format!(" (exit code {code})"),
        None => String::new(),
    }
}

fn map_spawn_error(err: std::io::Error) -> CliError {
    if err.kind() == std::io::ErrorKind::NotFound {
        return CliError::runtime(
            "pactl not found on PATH (install pipewire-pulse or pulseaudio-utils)",
        );
    }
    CliError::runtime(format!("failed to spawn pactl: {err}"))
}
