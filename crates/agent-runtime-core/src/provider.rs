//! Versioned provider adapter traits.
//!
//! `ProviderAdapterV1` is the stable provider-neutral interface consumed by
//! control-plane crates such as `agentctl`.

use crate::schema::{
    AuthStateRequest, AuthStateResponse, CapabilitiesRequest, CapabilitiesResponse, ExecuteRequest,
    ExecuteResponse, HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse,
    ProviderEnvelope, ProviderMetadata, ProviderOperation, ProviderResult,
};

/// Provider adapter contract version `v1`.
///
/// Implementors provide the five normalized adapter surfaces and return typed
/// results that are later wrapped into uniform envelopes for machine consumers.
pub trait ProviderAdapterV1: Send + Sync {
    /// Static provider metadata for contract negotiation and logging.
    fn metadata(&self) -> ProviderMetadata;

    /// Return provider capabilities and feature switches.
    fn capabilities(&self, request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse>;

    /// Probe provider readiness.
    fn healthcheck(&self, request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse>;

    /// Execute a provider-defined task payload.
    fn execute(&self, request: ExecuteRequest) -> ProviderResult<ExecuteResponse>;

    /// Report execution/auth/rate guardrails.
    fn limits(&self, request: LimitsRequest) -> ProviderResult<LimitsResponse>;

    /// Report current authentication state.
    fn auth_state(&self, request: AuthStateRequest) -> ProviderResult<AuthStateResponse>;

    /// Wrap `capabilities` result into the normalized response envelope.
    fn capabilities_envelope(
        &self,
        request: CapabilitiesRequest,
    ) -> ProviderEnvelope<CapabilitiesResponse> {
        ProviderEnvelope::from_result(
            self.metadata().provider_ref(),
            ProviderOperation::Capabilities,
            self.capabilities(request),
        )
    }

    /// Wrap `healthcheck` result into the normalized response envelope.
    fn healthcheck_envelope(
        &self,
        request: HealthcheckRequest,
    ) -> ProviderEnvelope<HealthcheckResponse> {
        ProviderEnvelope::from_result(
            self.metadata().provider_ref(),
            ProviderOperation::Healthcheck,
            self.healthcheck(request),
        )
    }

    /// Wrap `execute` result into the normalized response envelope.
    fn execute_envelope(&self, request: ExecuteRequest) -> ProviderEnvelope<ExecuteResponse> {
        ProviderEnvelope::from_result(
            self.metadata().provider_ref(),
            ProviderOperation::Execute,
            self.execute(request),
        )
    }

    /// Wrap `limits` result into the normalized response envelope.
    fn limits_envelope(&self, request: LimitsRequest) -> ProviderEnvelope<LimitsResponse> {
        ProviderEnvelope::from_result(
            self.metadata().provider_ref(),
            ProviderOperation::Limits,
            self.limits(request),
        )
    }

    /// Wrap `auth-state` result into the normalized response envelope.
    fn auth_state_envelope(
        &self,
        request: AuthStateRequest,
    ) -> ProviderEnvelope<AuthStateResponse> {
        ProviderEnvelope::from_result(
            self.metadata().provider_ref(),
            ProviderOperation::AuthState,
            self.auth_state(request),
        )
    }
}

/// Alias to the latest stable provider adapter contract.
pub trait ProviderAdapter: ProviderAdapterV1 {}

impl<T: ProviderAdapterV1 + ?Sized> ProviderAdapter for T {}
