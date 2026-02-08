use crate::provider::registry::{ProviderRegistry, ResolveProviderError};
use agent_runtime_core::schema::{HealthStatus, HealthcheckRequest};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 64;

#[derive(Debug, Args)]
pub struct ProviderArgs {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
}

#[derive(Debug, Subcommand)]
pub enum ProviderSubcommand {
    /// List registered providers and current health status
    List(ProviderListArgs),
    /// Execute healthcheck for one provider
    Healthcheck(ProviderHealthcheckArgs),
}

#[derive(Debug, Args)]
pub struct ProviderListArgs {
    /// Optional provider override (otherwise uses AGENTCTL_PROVIDER/default)
    #[arg(long)]
    pub provider: Option<String>,

    /// Healthcheck timeout passed to adapters
    #[arg(long)]
    pub timeout_ms: Option<u64>,

    /// Render format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ProviderHealthcheckArgs {
    /// Optional provider override (otherwise uses AGENTCTL_PROVIDER/default)
    #[arg(long)]
    pub provider: Option<String>,

    /// Healthcheck timeout passed to adapters
    #[arg(long)]
    pub timeout_ms: Option<u64>,

    /// Render format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

pub fn run(command: ProviderSubcommand) -> i32 {
    match command {
        ProviderSubcommand::List(args) => run_list(args),
        ProviderSubcommand::Healthcheck(args) => run_healthcheck(args),
    }
}

fn run_list(args: ProviderListArgs) -> i32 {
    let registry = ProviderRegistry::with_builtins();
    let selection = match registry.resolve_selection(args.provider.as_deref()) {
        Ok(selection) => selection,
        Err(error) => return report_selection_error(&registry, error),
    };
    let default_provider = registry.default_provider_id().map(ToOwned::to_owned);
    let selected_provider = selection.provider_id;
    let selected_source = selection.source.as_str().to_string();

    let providers = registry
        .iter()
        .map(|(provider_id, adapter)| {
            let metadata = adapter.metadata();
            let health = adapter.healthcheck(HealthcheckRequest {
                timeout_ms: args.timeout_ms,
            });
            let (status, summary) = match health {
                Ok(response) => (response.status, response.summary),
                Err(error) => (
                    HealthStatus::Unknown,
                    Some(format!("healthcheck failed: {}", error.message)),
                ),
            };

            ProviderListItem {
                id: provider_id.to_string(),
                contract_version: metadata.contract_version.as_str().to_string(),
                status: health_status_text(status).to_string(),
                summary,
                is_default: default_provider.as_deref() == Some(provider_id),
                is_selected: selected_provider.as_str() == provider_id,
            }
        })
        .collect::<Vec<_>>();

    let output = ProviderListOutput {
        default_provider,
        selected_provider,
        selected_source,
        providers,
    };

    emit_list_output(&output, args.format)
}

fn run_healthcheck(args: ProviderHealthcheckArgs) -> i32 {
    let registry = ProviderRegistry::with_builtins();
    let selection = match registry.resolve_selection(args.provider.as_deref()) {
        Ok(selection) => selection,
        Err(error) => return report_selection_error(&registry, error),
    };

    let Some(adapter) = registry.get(selection.provider_id.as_str()) else {
        eprintln!(
            "agentctl provider: selected provider '{}' is not registered",
            selection.provider_id
        );
        return EXIT_RUNTIME_ERROR;
    };

    let health = adapter.healthcheck(HealthcheckRequest {
        timeout_ms: args.timeout_ms,
    });
    match health {
        Ok(response) => {
            let output = ProviderHealthcheckOutput {
                provider: selection.provider_id,
                selected_source: selection.source.as_str().to_string(),
                status: health_status_text(response.status).to_string(),
                summary: response.summary,
                details: response.details,
            };
            emit_healthcheck_output(&output, args.format)
        }
        Err(error) => {
            if args.format == OutputFormat::Json {
                let output = ProviderHealthcheckFailureOutput {
                    provider: selection.provider_id,
                    selected_source: selection.source.as_str().to_string(),
                    error: ProviderCommandErrorOutput {
                        category: serde_json::to_value(error.category)
                            .ok()
                            .and_then(|value| value.as_str().map(ToOwned::to_owned))
                            .unwrap_or_else(|| "unknown".to_string()),
                        code: error.code,
                        message: error.message,
                    },
                };
                return emit_json(&output);
            }

            eprintln!(
                "agentctl provider: healthcheck failed for '{}': {}",
                selection.provider_id, error.message
            );
            EXIT_RUNTIME_ERROR
        }
    }
}

fn emit_list_output(output: &ProviderListOutput, format: OutputFormat) -> i32 {
    match format {
        OutputFormat::Json => emit_json(output),
        OutputFormat::Text => {
            println!(
                "default_provider: {}",
                output.default_provider.as_deref().unwrap_or("<none>")
            );
            println!(
                "selected_provider: {} ({})",
                output.selected_provider, output.selected_source
            );
            println!("providers:");
            for provider in &output.providers {
                let mut tags = Vec::new();
                if provider.is_default {
                    tags.push("default");
                }
                if provider.is_selected {
                    tags.push("selected");
                }

                if tags.is_empty() {
                    println!("- {} [{}]", provider.id, provider.status);
                } else {
                    println!(
                        "- {} [{}] ({})",
                        provider.id,
                        provider.status,
                        tags.join(", ")
                    );
                }
                if let Some(summary) = provider.summary.as_deref() {
                    println!("  summary: {summary}");
                }
            }
            EXIT_OK
        }
    }
}

fn emit_healthcheck_output(output: &ProviderHealthcheckOutput, format: OutputFormat) -> i32 {
    match format {
        OutputFormat::Json => emit_json(output),
        OutputFormat::Text => {
            println!("provider: {}", output.provider);
            println!("selected_source: {}", output.selected_source);
            println!("status: {}", output.status);
            if let Some(summary) = output.summary.as_deref() {
                println!("summary: {summary}");
            }
            EXIT_OK
        }
    }
}

fn emit_json<T: Serialize>(value: &T) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(encoded) => {
            println!("{encoded}");
            EXIT_OK
        }
        Err(error) => {
            eprintln!("agentctl provider: failed to render json output: {error}");
            EXIT_RUNTIME_ERROR
        }
    }
}

fn report_selection_error(registry: &ProviderRegistry, error: ResolveProviderError) -> i32 {
    eprintln!("agentctl provider: {error}");

    let registered = registry
        .iter()
        .map(|(provider_id, _)| provider_id.to_string())
        .collect::<Vec<_>>();
    if !registered.is_empty() {
        eprintln!(
            "agentctl provider: available providers: {}",
            registered.join(", ")
        );
    }

    EXIT_USAGE
}

fn health_status_text(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unhealthy => "unhealthy",
        HealthStatus::Unknown => "unknown",
    }
}

#[derive(Debug, Serialize)]
struct ProviderListOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    default_provider: Option<String>,
    selected_provider: String,
    selected_source: String,
    providers: Vec<ProviderListItem>,
}

#[derive(Debug, Serialize)]
struct ProviderListItem {
    id: String,
    contract_version: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    is_default: bool,
    is_selected: bool,
}

#[derive(Debug, Serialize)]
struct ProviderHealthcheckOutput {
    provider: String,
    selected_source: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ProviderHealthcheckFailureOutput {
    provider: String,
    selected_source: String,
    error: ProviderCommandErrorOutput,
}

#[derive(Debug, Serialize)]
struct ProviderCommandErrorOutput {
    category: String,
    code: String,
    message: String,
}
