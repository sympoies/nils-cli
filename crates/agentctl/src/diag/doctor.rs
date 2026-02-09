use crate::diag::{
    AutomationToolSpec, CheckStatus, Component, DIAG_SCHEMA_VERSION, EXIT_OK, EXIT_USAGE,
    FailureHint, FailureHintCategory, OutputFormat, ProbeMode, ProbeModeArg, ReadinessCheck,
    ReadinessSection, automation_tools, classify_hint_category, current_platform, emit_json,
    resolve_probe_mode,
};
use crate::provider::registry::ProviderRegistry;
use agent_runtime_core::schema::{
    HealthStatus, HealthcheckRequest, ProviderError, ProviderErrorCategory,
};
use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Optional provider filter (defaults to probing all registered providers)
    #[arg(long)]
    pub provider: Option<String>,

    /// Healthcheck timeout passed to provider adapters
    #[arg(long)]
    pub timeout_ms: Option<u64>,

    /// Render format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Probe execution mode (`test` enables deterministic CI probe behavior)
    #[arg(long, value_enum, default_value_t = ProbeModeArg::Auto)]
    pub probe_mode: ProbeModeArg,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    schema_version: &'static str,
    command: &'static str,
    probe_mode: ProbeMode,
    readiness: ReadinessSection,
}

pub fn run(args: DoctorArgs) -> i32 {
    let probe_mode = resolve_probe_mode(args.probe_mode);
    let readiness = match collect_readiness(args.provider.as_deref(), args.timeout_ms, probe_mode) {
        Ok(readiness) => readiness,
        Err(error) => {
            eprintln!("agentctl diag doctor: {error}");
            return EXIT_USAGE;
        }
    };

    let report = DoctorReport {
        schema_version: DIAG_SCHEMA_VERSION,
        command: "doctor",
        probe_mode,
        readiness,
    };

    match args.format {
        OutputFormat::Json => emit_json(&report),
        OutputFormat::Text => emit_text(&report),
    }
}

pub(crate) fn collect_readiness(
    provider_filter: Option<&str>,
    timeout_ms: Option<u64>,
    probe_mode: ProbeMode,
) -> Result<ReadinessSection, String> {
    let mut checks = collect_provider_checks(provider_filter, timeout_ms, probe_mode)?;
    checks.extend(collect_automation_checks(probe_mode));
    Ok(ReadinessSection::new(checks))
}

pub(crate) fn resolve_provider_ids(
    registry: &ProviderRegistry,
    provider_filter: Option<&str>,
) -> Result<Vec<String>, String> {
    if let Some(provider_id) = provider_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if registry.get(provider_id).is_none() {
            let available = registry
                .iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>();
            if available.is_empty() {
                return Err(format!(
                    "unknown provider '{}': no providers are registered",
                    provider_id
                ));
            }
            return Err(format!(
                "unknown provider '{}'. available providers: {}",
                provider_id,
                available.join(", ")
            ));
        }
        return Ok(vec![provider_id.to_string()]);
    }

    Ok(registry
        .iter()
        .map(|(provider_id, _)| provider_id.to_string())
        .collect::<Vec<_>>())
}

fn collect_provider_checks(
    provider_filter: Option<&str>,
    timeout_ms: Option<u64>,
    probe_mode: ProbeMode,
) -> Result<Vec<ReadinessCheck>, String> {
    let registry = ProviderRegistry::with_builtins();
    let provider_ids = resolve_provider_ids(&registry, provider_filter)?;

    let mut checks = Vec::with_capacity(provider_ids.len());
    for provider_id in provider_ids {
        let Some(adapter) = registry.get(provider_id.as_str()) else {
            continue;
        };

        let metadata = adapter.metadata();
        match adapter.healthcheck(HealthcheckRequest { timeout_ms }) {
            Ok(response) => {
                let status = map_health_status(response.status);
                let hint = provider_hint_from_health(
                    status,
                    response.summary.as_deref(),
                    response.details.as_ref(),
                );
                checks.push(ReadinessCheck {
                    id: format!("provider.{provider_id}.healthcheck"),
                    component: Component::Provider,
                    subject: provider_id,
                    probe: "healthcheck".to_string(),
                    status,
                    summary: response.summary,
                    hint,
                    details: Some(json!({
                        "contract_version": metadata.contract_version.as_str(),
                        "requested_timeout_ms": timeout_ms,
                        "provider_details": response.details,
                    })),
                    probe_mode,
                });
            }
            Err(error) => {
                let hint = provider_error_hint(error.as_ref());
                checks.push(ReadinessCheck {
                    id: format!("provider.{provider_id}.healthcheck"),
                    component: Component::Provider,
                    subject: provider_id,
                    probe: "healthcheck".to_string(),
                    status: CheckStatus::NotReady,
                    summary: Some(error.message.clone()),
                    hint,
                    details: Some(json!({
                        "category": provider_error_category(error.as_ref()),
                        "code": error.code,
                        "details": error.details,
                    })),
                    probe_mode,
                });
            }
        }
    }

    Ok(checks)
}

fn map_health_status(status: HealthStatus) -> CheckStatus {
    match status {
        HealthStatus::Healthy => CheckStatus::Ready,
        HealthStatus::Degraded => CheckStatus::Degraded,
        HealthStatus::Unhealthy => CheckStatus::NotReady,
        HealthStatus::Unknown => CheckStatus::Unknown,
    }
}

fn provider_hint_from_health(
    status: CheckStatus,
    summary: Option<&str>,
    details: Option<&Value>,
) -> Option<FailureHint> {
    if matches!(status, CheckStatus::Ready) {
        return None;
    }

    let mut combined = String::new();
    if let Some(summary) = summary {
        combined.push_str(summary);
    }
    if let Some(details) = details {
        if !combined.is_empty() {
            combined.push(' ');
        }
        combined.push_str(&details.to_string());
    }

    let category = classify_hint_category(&combined);
    if category == FailureHintCategory::Unknown {
        return None;
    }

    let message = summary
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("provider health is {}", status.as_str()));
    Some(FailureHint { category, message })
}

fn provider_error_hint(error: &ProviderError) -> Option<FailureHint> {
    let category = match error.category {
        ProviderErrorCategory::Dependency => FailureHintCategory::MissingDependency,
        ProviderErrorCategory::Auth => FailureHintCategory::Permission,
        ProviderErrorCategory::Unavailable => FailureHintCategory::PlatformLimitation,
        _ => classify_hint_category(&error.message),
    };

    if category == FailureHintCategory::Unknown {
        return None;
    }

    Some(FailureHint {
        category,
        message: error.message.clone(),
    })
}

fn provider_error_category(error: &ProviderError) -> String {
    serde_json::to_value(error.category)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_automation_checks(probe_mode: ProbeMode) -> Vec<ReadinessCheck> {
    automation_tools()
        .iter()
        .map(|spec| probe_automation_tool(spec, probe_mode))
        .collect::<Vec<_>>()
}

fn probe_automation_tool(spec: &AutomationToolSpec, probe_mode: ProbeMode) -> ReadinessCheck {
    let id = format!("automation.{}.readiness", spec.id);
    let command_path = find_command_path(spec.command);

    let Some(command_path) = command_path else {
        return ReadinessCheck {
            id,
            component: Component::Automation,
            subject: spec.id.to_string(),
            probe: "readiness".to_string(),
            status: CheckStatus::NotReady,
            summary: Some(format!("{} binary is missing from PATH", spec.command)),
            hint: Some(FailureHint {
                category: FailureHintCategory::MissingDependency,
                message: spec.install_hint.to_string(),
            }),
            details: Some(json!({
                "command": spec.command,
                "probe_args": spec.probe_args,
                "current_platform": current_platform(),
                "supported_platforms": spec.supported_platforms,
            })),
            probe_mode,
        };
    };

    if probe_mode == ProbeMode::Live && !supports_current_platform(spec) {
        let platform_message = if let Some(test_mode_env) = spec.test_mode_env {
            format!(
                "{} is not supported on {}. Use --probe-mode test or set {}=1 for deterministic CI probes.",
                spec.command,
                current_platform(),
                test_mode_env
            )
        } else {
            format!(
                "{} is not supported on {}.",
                spec.command,
                current_platform()
            )
        };

        return ReadinessCheck {
            id,
            component: Component::Automation,
            subject: spec.id.to_string(),
            probe: "readiness".to_string(),
            status: CheckStatus::NotReady,
            summary: Some(platform_message.clone()),
            hint: Some(FailureHint {
                category: FailureHintCategory::PlatformLimitation,
                message: platform_message,
            }),
            details: Some(json!({
                "command_path": command_path,
                "probe_args": spec.probe_args,
                "current_platform": current_platform(),
                "supported_platforms": spec.supported_platforms,
            })),
            probe_mode,
        };
    }

    let probe_output = match run_probe_command(spec, probe_mode) {
        Ok(output) => output,
        Err(error) => {
            return ReadinessCheck {
                id,
                component: Component::Automation,
                subject: spec.id.to_string(),
                probe: "readiness".to_string(),
                status: CheckStatus::NotReady,
                summary: Some(error),
                hint: Some(FailureHint {
                    category: FailureHintCategory::MissingDependency,
                    message: spec.install_hint.to_string(),
                }),
                details: Some(json!({
                    "command_path": command_path,
                    "probe_args": spec.probe_args,
                })),
                probe_mode,
            };
        }
    };

    if probe_output.success {
        if spec.id == "macos-agent"
            && let Some((status, summary, hint, details)) =
                parse_macos_agent_preflight(&probe_output.stdout)
        {
            return ReadinessCheck {
                id,
                component: Component::Automation,
                subject: spec.id.to_string(),
                probe: "readiness".to_string(),
                status,
                summary,
                hint,
                details: Some(details),
                probe_mode,
            };
        }

        return ReadinessCheck {
            id,
            component: Component::Automation,
            subject: spec.id.to_string(),
            probe: "readiness".to_string(),
            status: CheckStatus::Ready,
            summary: Some(format!("{} probe succeeded", spec.command)),
            hint: None,
            details: Some(json!({
                "command_path": command_path,
                "probe_args": spec.probe_args,
                "exit_code": probe_output.code,
            })),
            probe_mode,
        };
    }

    let combined = format!("{} {}", probe_output.stderr, probe_output.stdout);
    let category = classify_hint_category(&combined);
    let hint = if category == FailureHintCategory::Unknown {
        None
    } else {
        Some(FailureHint {
            category,
            message: automation_hint_message(spec, category),
        })
    };

    let summary = non_empty(&probe_output.stderr)
        .or_else(|| non_empty(&probe_output.stdout))
        .unwrap_or_else(|| {
            format!(
                "{} probe failed with exit code {}",
                spec.command, probe_output.code
            )
        });

    ReadinessCheck {
        id,
        component: Component::Automation,
        subject: spec.id.to_string(),
        probe: "readiness".to_string(),
        status: CheckStatus::NotReady,
        summary: Some(summary),
        hint,
        details: Some(json!({
            "command_path": command_path,
            "probe_args": spec.probe_args,
            "exit_code": probe_output.code,
            "stderr": probe_output.stderr,
            "stdout": probe_output.stdout,
        })),
        probe_mode,
    }
}

fn parse_macos_agent_preflight(
    stdout: &str,
) -> Option<(CheckStatus, Option<String>, Option<FailureHint>, Value)> {
    let parsed: Value = serde_json::from_str(stdout).ok()?;
    let ok = parsed.get("ok").and_then(Value::as_bool)?;
    if ok {
        return Some((
            CheckStatus::Ready,
            Some("macos-agent preflight checks passed".to_string()),
            None,
            parsed,
        ));
    }

    let mut status = CheckStatus::Degraded;
    let mut summary = None;
    let mut hint = None;
    let mut saw_issue = false;

    if let Some(checks) = parsed.pointer("/result/checks").and_then(Value::as_array) {
        for check in checks {
            let check_status = check
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if check_status == "ok" {
                continue;
            }

            saw_issue = true;
            let blocking = check
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if check_status == "fail" && blocking {
                status = CheckStatus::NotReady;
            }

            if summary.is_none() {
                summary = check
                    .get("message")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            if hint.is_none() {
                hint = macos_agent_hint_from_check(check);
            }
        }
    }

    if !saw_issue {
        status = CheckStatus::Unknown;
        summary = Some("macos-agent preflight returned non-ok status".to_string());
    }

    if hint.is_none() {
        let category = classify_hint_category(summary.as_deref().unwrap_or_default());
        if category != FailureHintCategory::Unknown {
            hint = Some(FailureHint {
                category,
                message: summary
                    .clone()
                    .unwrap_or_else(|| "macos-agent preflight reported warnings".to_string()),
            });
        }
    }

    if summary.is_none() {
        summary = Some("macos-agent preflight reported warnings".to_string());
    }

    Some((status, summary, hint, parsed))
}

fn macos_agent_hint_from_check(check: &Value) -> Option<FailureHint> {
    let check_id = check.get("id").and_then(Value::as_str).unwrap_or_default();
    let message = check
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let explicit_hint = check
        .get("hint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let category = match check_id {
        "osascript" | "cliclick" => FailureHintCategory::MissingDependency,
        "accessibility" | "automation" | "screen_recording" | "probe_activate"
        | "probe_input_hotkey" | "probe_screenshot" => FailureHintCategory::Permission,
        _ => classify_hint_category(format!("{message} {explicit_hint}").as_str()),
    };

    if category == FailureHintCategory::Unknown {
        return None;
    }

    let hint_message = non_empty(&explicit_hint)
        .or_else(|| non_empty(&message))
        .unwrap_or_else(|| automation_hint_message_for_category(category));
    Some(FailureHint {
        category,
        message: hint_message,
    })
}

fn supports_current_platform(spec: &AutomationToolSpec) -> bool {
    if spec.supported_platforms.is_empty() {
        return true;
    }

    spec.supported_platforms.contains(&current_platform())
}

fn run_probe_command(
    spec: &AutomationToolSpec,
    probe_mode: ProbeMode,
) -> Result<ProbeOutput, String> {
    let mut command = Command::new(spec.command);
    command.args(spec.probe_args);
    if probe_mode == ProbeMode::Test
        && let Some(test_mode_env) = spec.test_mode_env
    {
        command.env(test_mode_env, "1");
    }

    let output = command
        .output()
        .map_err(|error| format!("failed to execute `{}`: {error}", spec.command))?;

    Ok(ProbeOutput {
        success: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn automation_hint_message(spec: &AutomationToolSpec, category: FailureHintCategory) -> String {
    match category {
        FailureHintCategory::MissingDependency => spec.install_hint.to_string(),
        FailureHintCategory::Permission => format!(
            "Grant required OS permissions for `{}` and rerun diagnostics.",
            spec.command
        ),
        FailureHintCategory::PlatformLimitation => {
            format!(
                "`{}` probe is not supported on this platform.",
                spec.command
            )
        }
        FailureHintCategory::Unknown => automation_hint_message_for_category(category),
    }
}

fn automation_hint_message_for_category(category: FailureHintCategory) -> String {
    match category {
        FailureHintCategory::MissingDependency => {
            "Install required dependency and ensure PATH is configured.".to_string()
        }
        FailureHintCategory::Permission => {
            "Grant required OS permissions and rerun diagnostics.".to_string()
        }
        FailureHintCategory::PlatformLimitation => {
            "Run this probe on a supported platform.".to_string()
        }
        FailureHintCategory::Unknown => "Inspect probe stderr for details.".to_string(),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn emit_text(report: &DoctorReport) -> i32 {
    println!("schema_version: {}", report.schema_version);
    println!("command: {}", report.command);
    println!("probe_mode: {}", report.probe_mode.as_str());
    println!(
        "overall_status: {}",
        report.readiness.overall_status.as_str()
    );
    println!(
        "summary: total={} ready={} degraded={} not_ready={} unknown={}",
        report.readiness.summary.total_checks,
        report.readiness.summary.ready,
        report.readiness.summary.degraded,
        report.readiness.summary.not_ready,
        report.readiness.summary.unknown
    );
    println!("checks:");
    for check in &report.readiness.checks {
        println!(
            "- {}:{} [{}]",
            check.component.as_str(),
            check.subject,
            check.status.as_str()
        );
        if let Some(summary) = check.summary.as_deref() {
            println!("  summary: {summary}");
        }
        if let Some(hint) = check.hint.as_ref() {
            println!("  hint: {} ({})", hint.message, hint.category.as_str());
        }
    }

    EXIT_OK
}

fn find_command_path(command: &str) -> Option<String> {
    if command.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(command);
        if is_executable_file(&path) {
            return Some(path.display().to_string());
        }
        return None;
    }

    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable_file(candidate))
        .map(|candidate| candidate.display().to_string())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[derive(Debug)]
struct ProbeOutput {
    success: bool,
    code: i32,
    stdout: String,
    stderr: String,
}
