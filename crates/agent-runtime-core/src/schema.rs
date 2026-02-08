//! Versioned provider contract schema.
//!
//! This module defines the machine-readable contract shared between provider
//! adapters and `agentctl`:
//! - adapter operation payloads (`capabilities`, `healthcheck`, `execute`,
//!   `limits`, `auth-state`)
//! - normalized success/error envelopes
//! - stable error categorization and retry semantics

use serde::{Deserialize, Serialize};

/// Stable contract identifier for the v1 provider adapter schema.
pub const CONTRACT_VERSION_V1: &str = "provider-adapter.v1";

/// Supported provider contract versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ContractVersion {
    /// Current stable contract.
    #[default]
    #[serde(rename = "provider-adapter.v1")]
    V1,
}

impl ContractVersion {
    /// Return the stable wire value for this contract version.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => CONTRACT_VERSION_V1,
        }
    }
}

/// Provider identity included in each envelope for routing and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRef {
    pub id: String,
}

impl ProviderRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// Adapter metadata used by the provider trait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMetadata {
    pub id: String,
    #[serde(default)]
    pub contract_version: ContractVersion,
}

impl ProviderMetadata {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            contract_version: ContractVersion::V1,
        }
    }

    pub fn provider_ref(&self) -> ProviderRef {
        ProviderRef::new(self.id.clone())
    }
}

/// Standardized adapter operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderOperation {
    Capabilities,
    Healthcheck,
    Execute,
    Limits,
    AuthState,
}

/// Normalized provider result type used by contract APIs.
///
/// `ProviderError` is boxed to keep trait return types small and avoid
/// large-Err penalties in strict clippy configurations.
pub type ProviderResult<T> = std::result::Result<T, Box<ProviderError>>;

/// Normalized envelope that `agentctl` can consume uniformly across providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderEnvelope<T> {
    #[serde(default)]
    pub contract_version: ContractVersion,
    pub provider: ProviderRef,
    pub operation: ProviderOperation,
    #[serde(flatten)]
    pub outcome: ProviderOutcome<T>,
}

impl<T> ProviderEnvelope<T> {
    pub fn from_result(
        provider: ProviderRef,
        operation: ProviderOperation,
        result: ProviderResult<T>,
    ) -> Self {
        match result {
            Ok(payload) => Self::ok(provider, operation, payload),
            Err(error) => Self::error(provider, operation, *error),
        }
    }

    pub fn ok(provider: ProviderRef, operation: ProviderOperation, result: T) -> Self {
        Self {
            contract_version: ContractVersion::V1,
            provider,
            operation,
            outcome: ProviderOutcome::Ok { result },
        }
    }

    pub fn error(
        provider: ProviderRef,
        operation: ProviderOperation,
        error: ProviderError,
    ) -> Self {
        Self {
            contract_version: ContractVersion::V1,
            provider,
            operation,
            outcome: ProviderOutcome::Error { error },
        }
    }

    pub fn into_result(self) -> ProviderResult<T> {
        match self.outcome {
            ProviderOutcome::Ok { result } => Ok(result),
            ProviderOutcome::Error { error } => Err(Box::new(error)),
        }
    }
}

/// Tagged envelope outcome to keep success/error shape stable over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ProviderOutcome<T> {
    Ok { result: T },
    Error { error: ProviderError },
}

/// Stable error categorization for cross-provider policy handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderErrorCategory {
    Auth,
    RateLimit,
    Network,
    Timeout,
    Validation,
    Dependency,
    Unavailable,
    Internal,
    Unknown,
}

impl ProviderErrorCategory {
    /// Default retry policy by category.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Network | Self::Timeout | Self::Unavailable
        )
    }
}

/// Normalized provider error payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderError {
    pub category: ProviderErrorCategory,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ProviderError {
    pub fn new(
        category: ProviderErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            code: code.into(),
            message: message.into(),
            retryable: None,
            details: None,
        }
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = Some(retryable);
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
            .unwrap_or_else(|| self.category.is_retryable())
    }
}

/// Input payload for `capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilitiesRequest {
    #[serde(default)]
    pub include_experimental: bool,
}

/// Capability inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilitiesResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
}

/// Single declared capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Capability {
    pub fn available(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            available: true,
            description: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Input payload for `healthcheck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HealthcheckRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Provider health states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    #[default]
    Unknown,
}

/// Output payload for `healthcheck`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HealthcheckResponse {
    #[serde(default)]
    pub status: HealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Input payload for `execute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl ExecuteRequest {
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            input: None,
            timeout_ms: None,
        }
    }
}

/// Output payload for `execute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecuteResponse {
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Input payload for `limits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LimitsRequest {}

/// Output payload for `limits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LimitsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_bytes: Option<u64>,
}

/// Input payload for `auth-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthStateRequest {}

/// Authentication state categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStateStatus {
    Authenticated,
    Unauthenticated,
    Expired,
    #[default]
    Unknown,
}

/// Output payload for `auth-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthStateResponse {
    #[serde(default)]
    pub state: AuthStateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
