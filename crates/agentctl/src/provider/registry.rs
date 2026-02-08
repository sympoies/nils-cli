use agent_provider_codex::CodexProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use std::collections::BTreeMap;
use std::fmt;

pub const DEFAULT_PROVIDER_ID: &str = "codex";
pub const PROVIDER_OVERRIDE_ENV: &str = "AGENTCTL_PROVIDER";

pub struct ProviderRegistry {
    providers: BTreeMap<String, Box<dyn ProviderAdapterV1>>,
    default_provider_id: String,
}

impl ProviderRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::new(DEFAULT_PROVIDER_ID);
        registry.register(CodexProviderAdapter::new());
        registry
    }

    pub fn new(default_provider_id: impl Into<String>) -> Self {
        Self {
            providers: BTreeMap::new(),
            default_provider_id: default_provider_id.into(),
        }
    }

    pub fn register<T>(&mut self, adapter: T)
    where
        T: ProviderAdapterV1 + 'static,
    {
        let provider_id = adapter.metadata().id;
        self.providers.insert(provider_id, Box::new(adapter));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &dyn ProviderAdapterV1)> + '_ {
        self.providers
            .iter()
            .map(|(provider_id, adapter)| (provider_id.as_str(), adapter.as_ref()))
    }

    pub fn get(&self, provider_id: &str) -> Option<&dyn ProviderAdapterV1> {
        self.providers.get(provider_id).map(Box::as_ref)
    }

    pub fn default_provider_id(&self) -> Option<&str> {
        if self.providers.is_empty() {
            return None;
        }

        if self
            .providers
            .contains_key(self.default_provider_id.as_str())
        {
            return Some(self.default_provider_id.as_str());
        }

        self.providers.keys().next().map(String::as_str)
    }

    pub fn resolve_selection(
        &self,
        cli_override: Option<&str>,
    ) -> Result<ProviderSelection, ResolveProviderError> {
        let env_override = std::env::var(PROVIDER_OVERRIDE_ENV).ok();
        self.resolve_selection_with_env(cli_override, env_override.as_deref())
    }

    pub fn resolve_selection_with_env(
        &self,
        cli_override: Option<&str>,
        env_override: Option<&str>,
    ) -> Result<ProviderSelection, ResolveProviderError> {
        if let Some(provider_id) = normalize_provider_id(cli_override) {
            return self.resolve_override(provider_id, ProviderSelectionSource::CliArgument);
        }

        if let Some(provider_id) = normalize_provider_id(env_override) {
            return self.resolve_override(provider_id, ProviderSelectionSource::Environment);
        }

        let provider_id = self
            .default_provider_id()
            .ok_or(ResolveProviderError::NoProvidersRegistered)?;
        Ok(ProviderSelection {
            provider_id: provider_id.to_string(),
            source: ProviderSelectionSource::Default,
        })
    }

    fn resolve_override(
        &self,
        provider_id: &str,
        source: ProviderSelectionSource,
    ) -> Result<ProviderSelection, ResolveProviderError> {
        if !self.providers.contains_key(provider_id) {
            return Err(ResolveProviderError::UnknownProvider {
                provider_id: provider_id.to_string(),
                source,
            });
        }

        Ok(ProviderSelection {
            provider_id: provider_id.to_string(),
            source,
        })
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSelectionSource {
    CliArgument,
    Environment,
    Default,
}

impl ProviderSelectionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CliArgument => "cli-argument",
            Self::Environment => "environment",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSelection {
    pub provider_id: String,
    pub source: ProviderSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveProviderError {
    NoProvidersRegistered,
    UnknownProvider {
        provider_id: String,
        source: ProviderSelectionSource,
    },
}

impl fmt::Display for ResolveProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProvidersRegistered => f.write_str("no providers are registered"),
            Self::UnknownProvider {
                provider_id,
                source,
            } => write!(
                f,
                "unknown provider '{}' from {} override",
                provider_id,
                source.as_str()
            ),
        }
    }
}

impl std::error::Error for ResolveProviderError {}

fn normalize_provider_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}
