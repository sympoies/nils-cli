//! Repository security read arguments.
use clap::{Args, Subcommand, ValueEnum};

#[derive(Args, Debug)]
pub struct SecurityArgs {
    #[command(subcommand)]
    pub command: SecurityCommand,
}
#[derive(Subcommand, Debug)]
pub enum SecurityCommand {
    /// Read repository security alerts without scanned secret values.
    Alerts(SecurityAlertsArgs),
    /// Read repository security feature settings.
    Settings(SecuritySettingsArgs),
}
#[derive(Args, Debug)]
pub struct SecurityAlertsArgs {
    #[command(subcommand)]
    pub command: SecurityAlertsCommand,
}
#[derive(Subcommand, Debug)]
pub enum SecurityAlertsCommand {
    /// List alerts, bounded by --limit (default: 100).
    List(SecurityAlertListArgs),
    /// View code-scanning evidence for an exact ref and commit.
    View(SecurityAlertViewArgs),
}
#[derive(Args, Debug)]
pub struct SecuritySettingsArgs {
    #[command(subcommand)]
    pub command: SecuritySettingsCommand,
}
#[derive(Subcommand, Debug)]
pub enum SecuritySettingsCommand {
    /// View security_and_analysis; null means unavailable to this identity.
    View,
}
#[derive(Args, Debug)]
pub struct SecurityAlertListArgs {
    /// Security alert family.
    #[arg(long, value_enum)]
    pub kind: SecurityAlertKind,
    /// Provider state filter; all omits the state parameter.
    #[arg(long, default_value = "open", value_parser = ["open", "dismissed", "fixed", "auto_dismissed", "resolved", "all"])]
    pub state: String,
    /// Maximum alerts to return.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}
#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum SecurityAlertKind {
    Dependabot,
    CodeScanning,
    SecretScanning,
}
impl SecurityAlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dependabot => "dependabot",
            Self::CodeScanning => "code-scanning",
            Self::SecretScanning => "secret-scanning",
        }
    }
}

#[derive(Args, Debug)]
pub struct SecurityAlertViewArgs {
    /// Code-scanning alert number.
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub number: u64,
    /// Only code-scanning evidence is supported by this reader.
    #[arg(long, value_parser = ["code-scanning"])]
    pub kind: String,
    /// Exact Git reference, for example refs/pull/42/head.
    #[arg(long = "ref", value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub reference: String,
    /// Exact 40-character commit SHA for the requested instances.
    #[arg(long, value_parser = parse_commit)]
    pub commit: String,
    /// Fetch corresponding analysis SARIF, which may include source snippets.
    #[arg(long)]
    pub include_sarif: bool,
}

fn parse_commit(value: &str) -> Result<String, String> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err("expected a full 40-character commit SHA".into())
    }
}
