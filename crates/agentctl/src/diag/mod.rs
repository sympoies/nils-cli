pub mod capabilities;
pub mod doctor;

use clap::{Args, Subcommand, ValueEnum};
use nils_common::env as shared_env;
use serde::Serialize;

pub const DIAG_SCHEMA_VERSION: &str = "agentctl.diag.v1";
pub const EXIT_OK: i32 = 0;
pub const EXIT_RUNTIME_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 64;

pub const MACOS_AGENT_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_TEST_MODE";
pub const SCREEN_RECORD_TEST_MODE_ENV: &str = "CODEX_SCREEN_RECORD_TEST_MODE";

#[derive(Debug, Args)]
pub struct DiagArgs {
    #[command(subcommand)]
    pub command: Option<DiagSubcommand>,
}

#[derive(Debug, Subcommand)]
pub enum DiagSubcommand {
    /// Run provider and automation readiness diagnostics
    Doctor(doctor::DoctorArgs),
    /// Report provider and automation capability inventory
    Capabilities(capabilities::CapabilitiesArgs),
}

pub fn run(command: DiagSubcommand) -> i32 {
    match command {
        DiagSubcommand::Doctor(args) => doctor::run(args),
        DiagSubcommand::Capabilities(args) => capabilities::run(args),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ProbeModeArg {
    #[default]
    Auto,
    Live,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeMode {
    Live,
    Test,
}

impl ProbeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Test => "test",
        }
    }
}

pub fn resolve_probe_mode(mode: ProbeModeArg) -> ProbeMode {
    match mode {
        ProbeModeArg::Auto => {
            if shared_env::env_truthy(MACOS_AGENT_TEST_MODE_ENV)
                || shared_env::env_truthy(SCREEN_RECORD_TEST_MODE_ENV)
            {
                ProbeMode::Test
            } else {
                ProbeMode::Live
            }
        }
        ProbeModeArg::Live => ProbeMode::Live,
        ProbeModeArg::Test => ProbeMode::Test,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Component {
    Provider,
    Automation,
}

impl Component {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Automation => "automation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckStatus {
    Ready,
    Degraded,
    NotReady,
    Unknown,
}

impl CheckStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::NotReady => "not-ready",
            Self::Unknown => "unknown",
        }
    }

    const fn severity(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Unknown => 1,
            Self::Degraded => 2,
            Self::NotReady => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureHintCategory {
    MissingDependency,
    Permission,
    PlatformLimitation,
    Unknown,
}

impl FailureHintCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingDependency => "missing-dependency",
            Self::Permission => "permission",
            Self::PlatformLimitation => "platform-limitation",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailureHint {
    pub category: FailureHintCategory,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadinessCheck {
    pub id: String,
    pub component: Component,
    pub subject: String,
    pub probe: String,
    pub status: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<FailureHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub probe_mode: ProbeMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadinessSummary {
    pub total_checks: usize,
    pub ready: usize,
    pub degraded: usize,
    pub not_ready: usize,
    pub unknown: usize,
}

impl ReadinessSummary {
    pub fn from_checks(checks: &[ReadinessCheck]) -> Self {
        let mut ready = 0_usize;
        let mut degraded = 0_usize;
        let mut not_ready = 0_usize;
        let mut unknown = 0_usize;

        for check in checks {
            match check.status {
                CheckStatus::Ready => ready += 1,
                CheckStatus::Degraded => degraded += 1,
                CheckStatus::NotReady => not_ready += 1,
                CheckStatus::Unknown => unknown += 1,
            }
        }

        Self {
            total_checks: checks.len(),
            ready,
            degraded,
            not_ready,
            unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadinessSection {
    pub overall_status: CheckStatus,
    pub summary: ReadinessSummary,
    pub checks: Vec<ReadinessCheck>,
}

impl ReadinessSection {
    pub fn new(checks: Vec<ReadinessCheck>) -> Self {
        let overall_status = checks
            .iter()
            .map(|check| check.status)
            .max_by_key(|status| status.severity())
            .unwrap_or(CheckStatus::Unknown);
        let summary = ReadinessSummary::from_checks(&checks);

        Self {
            overall_status,
            summary,
            checks,
        }
    }
}

pub fn emit_json<T: Serialize>(value: &T) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(encoded) => {
            println!("{encoded}");
            EXIT_OK
        }
        Err(error) => {
            eprintln!("agentctl diag: failed to render json output: {error}");
            EXIT_RUNTIME_ERROR
        }
    }
}

pub fn classify_hint_category(text: &str) -> FailureHintCategory {
    let lower = text.to_ascii_lowercase();

    if lower.contains("permission")
        || lower.contains("not granted")
        || lower.contains("not allowed")
        || lower.contains("denied")
        || lower.contains("accessibility")
        || lower.contains("automation")
        || lower.contains("screen recording")
        || lower.contains("tcc")
    {
        return FailureHintCategory::Permission;
    }

    if lower.contains("unsupported platform")
        || lower.contains("only supported on")
        || lower.contains("not supported on this platform")
        || lower.contains("platform is unsupported")
    {
        return FailureHintCategory::PlatformLimitation;
    }

    if lower.contains("missing dependency")
        || lower.contains("not found in path")
        || lower.contains("command not found")
        || lower.contains("no such file or directory")
        || lower.contains("binary is missing")
        || lower.contains("is not available on path")
    {
        return FailureHintCategory::MissingDependency;
    }

    FailureHintCategory::Unknown
}

pub fn current_platform() -> &'static str {
    std::env::consts::OS
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AutomationToolSpec {
    pub id: &'static str,
    pub command: &'static str,
    pub probe_args: &'static [&'static str],
    pub supported_platforms: &'static [&'static str],
    pub test_mode_env: Option<&'static str>,
    pub install_hint: &'static str,
    pub capabilities: &'static [&'static str],
}

const MACOS_AGENT_CAPABILITIES: &[&str] = &[
    "preflight",
    "windows.list",
    "apps.list",
    "window.activate",
    "input.click",
    "input.type",
    "observe.screenshot",
];
const SCREEN_RECORD_CAPABILITIES: &[&str] = &[
    "preflight",
    "request-permission",
    "list-windows",
    "list-displays",
    "list-apps",
    "record",
    "screenshot",
];
const IMAGE_PROCESSING_CAPABILITIES: &[&str] = &[
    "info",
    "auto-orient",
    "convert",
    "resize",
    "rotate",
    "crop",
    "pad",
    "flip",
    "flop",
    "optimize",
];
const FZF_CLI_CAPABILITIES: &[&str] = &[
    "file",
    "directory",
    "git-status",
    "git-commit",
    "git-checkout",
    "git-branch",
    "git-tag",
    "process",
    "port",
    "history",
    "env",
    "alias",
    "function",
    "def",
];

pub(crate) const AUTOMATION_TOOL_SPECS: &[AutomationToolSpec] = &[
    AutomationToolSpec {
        id: "macos-agent",
        command: "macos-agent",
        probe_args: &["--format", "json", "preflight", "--strict"],
        supported_platforms: &["macos"],
        test_mode_env: Some(MACOS_AGENT_TEST_MODE_ENV),
        install_hint: "Install `macos-agent` and ensure the binary is discoverable on PATH.",
        capabilities: MACOS_AGENT_CAPABILITIES,
    },
    AutomationToolSpec {
        id: "screen-record",
        command: "screen-record",
        probe_args: &["--preflight"],
        supported_platforms: &["macos", "linux"],
        test_mode_env: Some(SCREEN_RECORD_TEST_MODE_ENV),
        install_hint: "Install `screen-record` and ensure the binary is discoverable on PATH.",
        capabilities: SCREEN_RECORD_CAPABILITIES,
    },
    AutomationToolSpec {
        id: "image-processing",
        command: "image-processing",
        probe_args: &["info", "--help"],
        supported_platforms: &[],
        test_mode_env: None,
        install_hint: "Install `image-processing` and ImageMagick (`magick`) support binaries.",
        capabilities: IMAGE_PROCESSING_CAPABILITIES,
    },
    AutomationToolSpec {
        id: "fzf-cli",
        command: "fzf-cli",
        probe_args: &["help"],
        supported_platforms: &[],
        test_mode_env: None,
        install_hint: "Install `fzf-cli` and required runtime helpers (`fzf`, `git`).",
        capabilities: FZF_CLI_CAPABILITIES,
    },
];

pub(crate) fn automation_tools() -> &'static [AutomationToolSpec] {
    AUTOMATION_TOOL_SPECS
}
