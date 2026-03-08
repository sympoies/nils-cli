use std::sync::atomic::AtomicBool;

use nils_common::provider_runtime::{
    ExecInvocation, ExecProfile, HomePathSelection, PathsProfile, ProviderDefaults,
    ProviderEnvKeys, ProviderProfile,
};

const SECRET_HOME_MODERN: &[&str] = &[".gemini", "secrets"];
const AUTH_HOME_MODERN: &[&str] = &[".gemini", "oauth_creds.json"];
const CACHE_HOME: &[&str] = &[".gemini", "cache", "secrets"];

static WARNED_INVALID_ALLOW_DANGEROUS: AtomicBool = AtomicBool::new(false);

pub static GEMINI_PROVIDER_PROFILE: ProviderProfile = ProviderProfile {
    provider_name: "gemini",
    env: ProviderEnvKeys {
        model: "GEMINI_CLI_MODEL",
        reasoning: "GEMINI_CLI_REASONING",
        allow_dangerous_enabled: "GEMINI_ALLOW_DANGEROUS_ENABLED",
        secret_dir: "GEMINI_SECRET_DIR",
        auth_file: "GEMINI_AUTH_FILE",
        secret_cache_dir: "GEMINI_SECRET_CACHE_DIR",
        prompt_segment_enabled: "GEMINI_PROMPT_SEGMENT_ENABLED",
        auto_refresh_enabled: "GEMINI_AUTO_REFRESH_ENABLED",
        auto_refresh_min_days: "GEMINI_AUTO_REFRESH_MIN_DAYS",
    },
    defaults: ProviderDefaults {
        model: "gemini-2.5-flash",
        reasoning: "medium",
        prompt_segment_enabled: "false",
        auto_refresh_enabled: "false",
        auto_refresh_min_days: "5",
    },
    paths: PathsProfile {
        feature_name: "gemini",
        feature_tool_script: "gemini-tools.zsh",
        secret_dir_home: HomePathSelection::ModernOnly(SECRET_HOME_MODERN),
        auth_file_home: HomePathSelection::ModernOnly(AUTH_HOME_MODERN),
        secret_cache_home: Some(CACHE_HOME),
    },
    exec: ExecProfile {
        default_caller_prefix: "gemini",
        missing_prompt_label: "_gemini_exec_dangerous",
        binary_name: "gemini",
        failed_exec_message_prefix: "gemini-tools: failed to run gemini exec",
        invocation: ExecInvocation::GeminiStyle,
        warned_invalid_allow_dangerous: &WARNED_INVALID_ALLOW_DANGEROUS,
    },
};
