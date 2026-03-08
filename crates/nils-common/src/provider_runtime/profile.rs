use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy)]
pub struct ProviderProfile {
    pub provider_name: &'static str,
    pub env: ProviderEnvKeys,
    pub defaults: ProviderDefaults,
    pub paths: PathsProfile,
    pub exec: ExecProfile,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderEnvKeys {
    pub model: &'static str,
    pub reasoning: &'static str,
    pub allow_dangerous_enabled: &'static str,
    pub secret_dir: &'static str,
    pub auth_file: &'static str,
    pub secret_cache_dir: &'static str,
    pub prompt_segment_enabled: &'static str,
    pub auto_refresh_enabled: &'static str,
    pub auto_refresh_min_days: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDefaults {
    pub model: &'static str,
    pub reasoning: &'static str,
    pub prompt_segment_enabled: &'static str,
    pub auto_refresh_enabled: &'static str,
    pub auto_refresh_min_days: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct PathsProfile {
    pub feature_name: &'static str,
    pub feature_tool_script: &'static str,
    pub secret_dir_home: HomePathSelection,
    pub auth_file_home: HomePathSelection,
    pub secret_cache_home: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone, Copy)]
pub enum HomePathSelection {
    ModernOnly(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub struct ExecProfile {
    pub default_caller_prefix: &'static str,
    pub missing_prompt_label: &'static str,
    pub binary_name: &'static str,
    pub failed_exec_message_prefix: &'static str,
    pub invocation: ExecInvocation,
    pub warned_invalid_allow_dangerous: &'static AtomicBool,
}

#[derive(Debug, Clone, Copy)]
pub enum ExecInvocation {
    CodexStyle,
    GeminiStyle,
}
