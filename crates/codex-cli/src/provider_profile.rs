use std::sync::atomic::AtomicBool;

use nils_common::provider_runtime::{
    ExecInvocation, ExecProfile, HomePathSelection, PathsProfile, ProviderDefaults,
    ProviderEnvKeys, ProviderProfile,
};

const SECRET_HOME: &[&str] = &[".config", "codex_secrets"];
const AUTH_HOME: &[&str] = &[".agents", "auth.json"];

static WARNED_INVALID_ALLOW_DANGEROUS: AtomicBool = AtomicBool::new(false);

pub static CODEX_PROVIDER_PROFILE: ProviderProfile = ProviderProfile {
    provider_name: "codex",
    env: ProviderEnvKeys {
        model: "CODEX_CLI_MODEL",
        reasoning: "CODEX_CLI_REASONING",
        allow_dangerous_enabled: "CODEX_ALLOW_DANGEROUS_ENABLED",
        secret_dir: "CODEX_SECRET_DIR",
        auth_file: "CODEX_AUTH_FILE",
        secret_cache_dir: "CODEX_SECRET_CACHE_DIR",
        prompt_segment_enabled: "CODEX_PROMPT_SEGMENT_ENABLED",
        auto_refresh_enabled: "CODEX_AUTO_REFRESH_ENABLED",
        auto_refresh_min_days: "CODEX_AUTO_REFRESH_MIN_DAYS",
    },
    defaults: ProviderDefaults {
        model: "gpt-5.1-codex-mini",
        reasoning: "medium",
        prompt_segment_enabled: "false",
        auto_refresh_enabled: "false",
        auto_refresh_min_days: "5",
    },
    paths: PathsProfile {
        feature_name: "codex",
        feature_tool_script: "codex-tools.zsh",
        secret_dir_home: HomePathSelection::ModernOnly(SECRET_HOME),
        auth_file_home: HomePathSelection::ModernOnly(AUTH_HOME),
        secret_cache_home: None,
    },
    exec: ExecProfile {
        default_caller_prefix: "codex",
        missing_prompt_label: "_codex_exec_dangerous",
        binary_name: "codex",
        failed_exec_message_prefix: "codex-tools: failed to run codex exec",
        invocation: ExecInvocation::CodexStyle,
        warned_invalid_allow_dangerous: &WARNED_INVALID_ALLOW_DANGEROUS,
    },
};
