use crate::diag::{
    AutomationToolSpec, Component, DIAG_SCHEMA_VERSION, EXIT_OK, EXIT_USAGE, OutputFormat,
    ProbeMode, ProbeModeArg, ReadinessSection, automation_tools, current_platform, doctor,
    emit_json, resolve_probe_mode,
};
use crate::provider::registry::ProviderRegistry;
use agent_runtime_core::schema::CapabilitiesRequest;
use clap::Args;
use serde::Serialize;

#[derive(Debug, Args)]
pub struct CapabilitiesArgs {
    /// Optional provider filter (defaults to querying all registered providers)
    #[arg(long)]
    pub provider: Option<String>,

    /// Include experimental capability flags from provider adapters
    #[arg(long)]
    pub include_experimental: bool,

    /// Healthcheck timeout passed to readiness checks
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
struct CapabilitiesReport {
    schema_version: &'static str,
    command: &'static str,
    probe_mode: ProbeMode,
    readiness: ReadinessSection,
    providers: Vec<ProviderCapabilities>,
    automation_tools: Vec<AutomationToolCapabilities>,
}

#[derive(Debug, Serialize)]
struct ProviderCapabilities {
    id: String,
    contract_version: String,
    capabilities: Vec<CapabilityEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ProviderCapabilitiesError>,
}

#[derive(Debug, Serialize)]
struct CapabilityEntry {
    name: String,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProviderCapabilitiesError {
    category: String,
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct AutomationToolCapabilities {
    id: String,
    command: String,
    capabilities: Vec<String>,
    supported_platforms: Vec<String>,
    supports_current_platform: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_mode_env: Option<String>,
}

pub fn run(args: CapabilitiesArgs) -> i32 {
    let probe_mode = resolve_probe_mode(args.probe_mode);
    let readiness =
        match doctor::collect_readiness(args.provider.as_deref(), args.timeout_ms, probe_mode) {
            Ok(readiness) => readiness,
            Err(error) => {
                eprintln!("agentctl diag capabilities: {error}");
                return EXIT_USAGE;
            }
        };

    let providers =
        match collect_provider_capabilities(args.provider.as_deref(), args.include_experimental) {
            Ok(providers) => providers,
            Err(error) => {
                eprintln!("agentctl diag capabilities: {error}");
                return EXIT_USAGE;
            }
        };

    let automation_tools = collect_automation_capabilities();
    let report = CapabilitiesReport {
        schema_version: DIAG_SCHEMA_VERSION,
        command: "capabilities",
        probe_mode,
        readiness,
        providers,
        automation_tools,
    };

    match args.format {
        OutputFormat::Json => emit_json(&report),
        OutputFormat::Text => emit_text(&report),
    }
}

fn collect_provider_capabilities(
    provider_filter: Option<&str>,
    include_experimental: bool,
) -> Result<Vec<ProviderCapabilities>, String> {
    let registry = ProviderRegistry::with_builtins();
    let provider_ids = doctor::resolve_provider_ids(&registry, provider_filter)?;
    let mut providers = Vec::with_capacity(provider_ids.len());

    for provider_id in provider_ids {
        let Some(adapter) = registry.get(provider_id.as_str()) else {
            continue;
        };
        let metadata = adapter.metadata();
        match adapter.capabilities(CapabilitiesRequest {
            include_experimental,
        }) {
            Ok(response) => {
                let capabilities = response
                    .capabilities
                    .into_iter()
                    .map(|capability| CapabilityEntry {
                        name: capability.name,
                        available: capability.available,
                        description: capability.description,
                    })
                    .collect::<Vec<_>>();
                providers.push(ProviderCapabilities {
                    id: provider_id,
                    contract_version: metadata.contract_version.as_str().to_string(),
                    capabilities,
                    error: None,
                });
            }
            Err(error) => {
                let category = serde_json::to_value(error.category)
                    .ok()
                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "unknown".to_string());
                providers.push(ProviderCapabilities {
                    id: provider_id,
                    contract_version: metadata.contract_version.as_str().to_string(),
                    capabilities: Vec::new(),
                    error: Some(ProviderCapabilitiesError {
                        category,
                        code: error.code,
                        message: error.message,
                    }),
                });
            }
        }
    }

    Ok(providers)
}

fn collect_automation_capabilities() -> Vec<AutomationToolCapabilities> {
    automation_tools()
        .iter()
        .map(|spec| AutomationToolCapabilities {
            id: spec.id.to_string(),
            command: spec.command.to_string(),
            capabilities: spec
                .capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            supported_platforms: spec
                .supported_platforms
                .iter()
                .map(|platform| platform.to_string())
                .collect(),
            supports_current_platform: supports_current_platform(spec),
            test_mode_env: spec.test_mode_env.map(ToOwned::to_owned),
        })
        .collect()
}

fn supports_current_platform(spec: &AutomationToolSpec) -> bool {
    if spec.supported_platforms.is_empty() {
        return true;
    }

    spec.supported_platforms.contains(&current_platform())
}

fn provider_readiness_reason(
    report: &CapabilitiesReport,
    provider_id: &str,
) -> Option<(String, String)> {
    report
        .readiness
        .checks
        .iter()
        .find(|check| check.component == Component::Provider && check.subject == provider_id)
        .and_then(|check| doctor::readiness_reason_fields(check.details.as_ref()))
}

fn emit_text(report: &CapabilitiesReport) -> i32 {
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
    println!("providers:");
    for provider in &report.providers {
        println!("- {} ({})", provider.id, provider.contract_version);
        if let Some(error) = provider.error.as_ref() {
            println!(
                "  error: {} [{}:{}]",
                error.message, error.category, error.code
            );
            continue;
        }
        if let Some((code, message)) = provider_readiness_reason(report, provider.id.as_str()) {
            println!("  readiness_reason: {code} ({message})");
        }
        for capability in &provider.capabilities {
            if let Some(description) = capability.description.as_deref() {
                println!(
                    "  - {} [{}] {}",
                    capability.name,
                    if capability.available {
                        "available"
                    } else {
                        "unavailable"
                    },
                    description
                );
            } else {
                println!(
                    "  - {} [{}]",
                    capability.name,
                    if capability.available {
                        "available"
                    } else {
                        "unavailable"
                    }
                );
            }
        }
    }
    println!("automation_tools:");
    for tool in &report.automation_tools {
        println!(
            "- {} ({}) [{}]",
            tool.id,
            tool.command,
            if tool.supports_current_platform {
                "supported"
            } else {
                "unsupported"
            }
        );
        println!("  capabilities: {}", tool.capabilities.join(", "));
        if let Some(test_mode_env) = tool.test_mode_env.as_deref() {
            println!("  test_mode_env: {test_mode_env}");
        }
    }

    EXIT_OK
}
