use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Tsv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ErrorFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImageFormat {
    Png,
    #[value(alias = "jpeg")]
    Jpg,
    Webp,
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "macos-agent",
    version,
    about = "Automate macOS desktop actions for agent workflows.",
    after_help = "Decision guide (AX-first):\n\
1) Prefer `ax` commands for element-targeted interaction.\n\
2) If AX is flaky, enable fallback flags (`--allow-coordinate-fallback`, `--allow-keyboard-fallback`).\n\
3) If AX is unavailable, use `window activate` + `input` commands.\n\
4) Add `wait` commands around mutating steps for stability.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    pub format: OutputFormat,

    /// Error output format.
    #[arg(long, value_enum, default_value_t = ErrorFormat::Text, global = true)]
    pub error_format: ErrorFormat,

    /// Print planned actions without mutating desktop state.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Retry count for mutating actions.
    #[arg(long, default_value_t = 0, global = true)]
    pub retries: u8,

    /// Delay between retries in milliseconds.
    #[arg(long, default_value_t = 150, global = true)]
    pub retry_delay_ms: u64,

    /// Per-action timeout in milliseconds.
    #[arg(long, default_value_t = 4000, global = true)]
    pub timeout_ms: u64,

    /// Emit per-command trace artifacts under CODEX_HOME/out.
    #[arg(long, global = true)]
    pub trace: bool,

    /// Override trace output directory (requires --trace).
    #[arg(long, global = true)]
    pub trace_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: CommandGroup,
}

#[derive(Debug, Clone, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum CommandGroup {
    /// Check runtime dependencies and permissions.
    Preflight(PreflightArgs),

    /// List windows.
    Windows {
        #[command(subcommand)]
        command: WindowsCommand,
    },

    /// List running apps.
    Apps {
        #[command(subcommand)]
        command: AppsCommand,
    },

    /// Activate a target window/app.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },

    /// Execute pointer and keyboard input commands.
    Input {
        #[command(subcommand)]
        command: InputCommand,
    },

    /// Query and switch macOS keyboard input sources.
    InputSource {
        #[command(subcommand)]
        command: InputSourceCommand,
    },

    /// Query and interact with Accessibility (AX) nodes.
    Ax {
        #[command(subcommand)]
        command: AxCommand,
    },

    /// Capture screenshots for observation.
    Observe {
        #[command(subcommand)]
        command: ObserveCommand,
    },

    /// Wait primitives for UI stabilization.
    Wait {
        #[command(subcommand)]
        command: WaitCommand,
    },

    /// Run declarative multi-step command chains.
    Scenario {
        #[command(subcommand)]
        command: ScenarioCommand,
    },

    /// Validate and bootstrap coordinate profiles.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Debug, Clone, Args)]
pub struct PreflightArgs {
    /// Treat advisory warnings as readiness failures.
    #[arg(long)]
    pub strict: bool,

    /// Run actionable probes (activate/input/screenshot) in addition to static checks.
    #[arg(long)]
    pub include_probes: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowsCommand {
    /// List windows.
    List(ListWindowsArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ListWindowsArgs {
    /// Filter by app/owner name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow app selection by window title substring.
    #[arg(
        long = "window-title-contains",
        visible_alias = "window-name",
        requires = "app"
    )]
    pub window_name: Option<String>,

    /// Include only on-screen windows.
    #[arg(long)]
    pub on_screen_only: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AppsCommand {
    /// List running apps.
    List(ListAppsArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct ListAppsArgs {}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowCommand {
    /// Activate a target window/app.
    Activate(WindowActivateArgs),
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(false)
            .args(["window_id", "active_window", "app", "bundle_id"])
    )
)]
pub struct WindowActivateArgs {
    /// Select by window id.
    #[arg(long)]
    pub window_id: Option<u32>,

    /// Select frontmost active window.
    #[arg(long)]
    pub active_window: bool,

    /// Select by app name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow app selection by window title substring.
    #[arg(
        long = "window-title-contains",
        visible_alias = "window-name",
        requires = "app"
    )]
    pub window_name: Option<String>,

    /// Select by bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Wait up to this many milliseconds for active confirmation.
    #[arg(long)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputCommand {
    /// Click at x/y coordinates.
    Click(InputClickArgs),

    /// Type text.
    Type(InputTypeArgs),

    /// Send hotkey chord.
    Hotkey(InputHotkeyArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputSourceCommand {
    /// Show current keyboard input source id.
    Current(InputSourceCurrentArgs),

    /// Switch to a keyboard input source id.
    Switch(InputSourceSwitchArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct InputSourceCurrentArgs {}

#[derive(Debug, Clone, Args)]
pub struct InputSourceSwitchArgs {
    /// Input source id or alias (`abc`, `us`, or full source id).
    #[arg(long)]
    pub id: String,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AxCommand {
    /// List AX nodes.
    List(AxListArgs),

    /// Click an AX node.
    Click(AxClickArgs),

    /// Type text into an AX node.
    Type(AxTypeArgs),

    /// Read or set arbitrary AX attributes.
    Attr {
        #[command(subcommand)]
        command: AxAttrCommand,
    },

    /// Perform arbitrary AX actions.
    Action {
        #[command(subcommand)]
        command: AxActionCommand,
    },

    /// Manage long-lived AX sessions.
    Session {
        #[command(subcommand)]
        command: AxSessionCommand,
    },

    /// Start/poll/stop AX notification watchers.
    Watch {
        #[command(subcommand)]
        command: AxWatchCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AxAttrCommand {
    /// Read an AX attribute value.
    Get(AxAttrGetArgs),

    /// Set an AX attribute value.
    Set(AxAttrSetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AxActionCommand {
    /// Perform an AX action.
    Perform(AxActionPerformArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AxSessionCommand {
    /// Create or update an AX session.
    Start(AxSessionStartArgs),

    /// List active AX sessions.
    List(AxSessionListArgs),

    /// Stop an AX session.
    Stop(AxSessionStopArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AxWatchCommand {
    /// Start a watcher bound to an AX session.
    Start(AxWatchStartArgs),

    /// Poll buffered watcher events.
    Poll(AxWatchPollArgs),

    /// Stop a watcher.
    Stop(AxWatchStopArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct AxTargetArgs {
    /// Select target by existing session id.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Select target app by name.
    #[arg(long)]
    pub app: Option<String>,

    /// Select target app by bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Filter roots by window title substring.
    #[arg(long)]
    pub window_title_contains: Option<String>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct AxMatchFiltersArgs {
    /// Select by AX role.
    #[arg(long)]
    pub role: Option<String>,

    /// Select by title substring.
    #[arg(long)]
    pub title_contains: Option<String>,

    /// Select by identifier substring.
    #[arg(long)]
    pub identifier_contains: Option<String>,

    /// Select by value substring.
    #[arg(long)]
    pub value_contains: Option<String>,

    /// Select by AX subrole.
    #[arg(long)]
    pub subrole: Option<String>,

    /// Select by focused state.
    #[arg(long)]
    pub focused: Option<bool>,

    /// Select by enabled state.
    #[arg(long)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct AxSelectorArgs {
    /// Select by node id from `ax list`.
    #[arg(
        long,
        conflicts_with_all = [
            "role",
            "title_contains",
            "identifier_contains",
            "value_contains",
            "subrole",
            "focused",
            "enabled",
            "nth"
        ]
    )]
    pub node_id: Option<String>,

    #[command(flatten)]
    pub filters: AxMatchFiltersArgs,

    /// Select the nth match from compound selector results.
    #[arg(long)]
    pub nth: Option<u32>,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxListArgs {
    #[command(flatten)]
    pub target: AxTargetArgs,

    #[command(flatten)]
    pub filters: AxMatchFiltersArgs,

    /// Limit traversal depth.
    #[arg(long)]
    pub max_depth: Option<u32>,

    /// Limit number of returned nodes.
    #[arg(long)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args([
                "node_id",
                "role",
                "title_contains",
                "identifier_contains",
                "value_contains",
                "subrole",
                "focused",
                "enabled",
            ])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxClickArgs {
    #[command(flatten)]
    pub selector: AxSelectorArgs,

    #[command(flatten)]
    pub target: AxTargetArgs,

    /// Allow coordinate fallback when AX press is unavailable.
    #[arg(long)]
    pub allow_coordinate_fallback: bool,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args([
                "node_id",
                "role",
                "title_contains",
                "identifier_contains",
                "value_contains",
                "subrole",
                "focused",
                "enabled",
            ])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxTypeArgs {
    #[command(flatten)]
    pub selector: AxSelectorArgs,

    #[command(flatten)]
    pub target: AxTargetArgs,

    /// Text to type.
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub text: String,

    /// Clear field value before typing.
    #[arg(long)]
    pub clear_first: bool,

    /// Submit (Enter) after typing.
    #[arg(long)]
    pub submit: bool,

    /// Use clipboard paste strategy.
    #[arg(long)]
    pub paste: bool,

    /// Allow keyboard fallback when AX value set/focus is unavailable.
    #[arg(long)]
    pub allow_keyboard_fallback: bool,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args([
                "node_id",
                "role",
                "title_contains",
                "identifier_contains",
                "value_contains",
                "subrole",
                "focused",
                "enabled",
            ])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxAttrGetArgs {
    #[command(flatten)]
    pub selector: AxSelectorArgs,

    #[command(flatten)]
    pub target: AxTargetArgs,

    /// AX attribute name.
    #[arg(long)]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AxValueType {
    String,
    Number,
    Bool,
    Json,
    Null,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args([
                "node_id",
                "role",
                "title_contains",
                "identifier_contains",
                "value_contains",
                "subrole",
                "focused",
                "enabled",
            ])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxAttrSetArgs {
    #[command(flatten)]
    pub selector: AxSelectorArgs,

    #[command(flatten)]
    pub target: AxTargetArgs,

    /// AX attribute name.
    #[arg(long)]
    pub name: String,

    /// AX attribute value.
    #[arg(long)]
    pub value: String,

    /// Parse type for --value.
    #[arg(long, value_enum, default_value_t = AxValueType::String)]
    pub value_type: AxValueType,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args([
                "node_id",
                "role",
                "title_contains",
                "identifier_contains",
                "value_contains",
                "subrole",
                "focused",
                "enabled",
            ])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["session_id", "app", "bundle_id"])
    )
)]
pub struct AxActionPerformArgs {
    #[command(flatten)]
    pub selector: AxSelectorArgs,

    #[command(flatten)]
    pub target: AxTargetArgs,

    /// AX action name.
    #[arg(long)]
    pub name: String,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["app", "bundle_id"])
    )
)]
pub struct AxSessionStartArgs {
    /// Select target app by name.
    #[arg(long)]
    pub app: Option<String>,

    /// Select target app by bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Optionally name the session id.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Persist window title root filter in this session.
    #[arg(long)]
    pub window_title_contains: Option<String>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct AxSessionListArgs {}

#[derive(Debug, Clone, Args)]
pub struct AxSessionStopArgs {
    /// Session id to remove.
    #[arg(long)]
    pub session_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct AxWatchStartArgs {
    /// Session id to bind watcher to.
    #[arg(long)]
    pub session_id: String,

    /// Optional watcher id.
    #[arg(long)]
    pub watch_id: Option<String>,

    /// Comma-separated AX notification names.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "AXFocusedUIElementChanged,AXTitleChanged"
    )]
    pub events: Vec<String>,

    /// Maximum in-memory event buffer size.
    #[arg(long, default_value_t = 256)]
    pub max_buffer: usize,
}

#[derive(Debug, Clone, Args)]
pub struct AxWatchPollArgs {
    /// Watch id to poll.
    #[arg(long)]
    pub watch_id: String,

    /// Max events to return.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    /// Drain returned events from watcher buffer.
    #[arg(long, default_value_t = true)]
    pub drain: bool,
}

#[derive(Debug, Clone, Args)]
pub struct AxWatchStopArgs {
    /// Watch id to stop.
    #[arg(long)]
    pub watch_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct InputClickArgs {
    /// X coordinate in pixels.
    #[arg(long)]
    pub x: i32,

    /// Y coordinate in pixels.
    #[arg(long)]
    pub y: i32,

    /// Mouse button.
    #[arg(long, value_enum, default_value_t = MouseButton::Left)]
    pub button: MouseButton,

    /// Number of clicks.
    #[arg(long, default_value_t = 1)]
    pub count: u8,

    /// Wait before clicking.
    #[arg(long, default_value_t = 0)]
    pub pre_wait_ms: u64,

    /// Wait after clicking.
    #[arg(long, default_value_t = 0)]
    pub post_wait_ms: u64,
}

#[derive(Debug, Clone, Args)]
pub struct InputTypeArgs {
    /// Text to type.
    #[arg(long)]
    pub text: String,

    /// Delay between key events.
    #[arg(long)]
    pub delay_ms: Option<u64>,

    /// Press Enter after typing.
    #[arg(long = "submit", visible_alias = "enter")]
    pub enter: bool,
}

#[derive(Debug, Clone, Args)]
pub struct InputHotkeyArgs {
    /// Modifier keys, comma-separated.
    #[arg(long)]
    pub mods: String,

    /// Main key.
    #[arg(long)]
    pub key: String,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ObserveCommand {
    /// Capture a screenshot.
    Screenshot(ObserveScreenshotArgs),
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(false)
            .args(["window_id", "active_window", "app"])
    )
)]
pub struct ObserveScreenshotArgs {
    /// Select by window id.
    #[arg(long)]
    pub window_id: Option<u32>,

    /// Select frontmost active window.
    #[arg(long)]
    pub active_window: bool,

    /// Select by app name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow app selection by window title substring.
    #[arg(
        long = "window-title-contains",
        visible_alias = "window-name",
        requires = "app"
    )]
    pub window_name: Option<String>,

    /// Output path.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Output image format.
    #[arg(long, value_enum)]
    pub image_format: Option<ImageFormat>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WaitCommand {
    /// Sleep for a fixed duration.
    Sleep(WaitSleepArgs),

    /// Wait for app to become active.
    AppActive(WaitAppActiveArgs),

    /// Wait for target window to appear.
    WindowPresent(WaitWindowPresentArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ScenarioCommand {
    /// Run steps from a scenario JSON file.
    Run(ScenarioRunArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ScenarioRunArgs {
    /// Scenario JSON file path.
    #[arg(long)]
    pub file: PathBuf,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProfileCommand {
    /// Validate profile schema and coordinate bounds.
    Validate(ProfileValidateArgs),
    /// Write a scaffold profile template.
    Init(ProfileInitArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ProfileValidateArgs {
    /// Profile JSON file path.
    #[arg(long)]
    pub file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct ProfileInitArgs {
    /// Profile name to embed in generated scaffold.
    #[arg(long, default_value = "default-1440p")]
    pub name: String,

    /// Output path for scaffold JSON.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct WaitSleepArgs {
    /// Sleep duration in milliseconds.
    #[arg(long)]
    pub ms: u64,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(false)
            .args(["app", "bundle_id"])
    )
)]
pub struct WaitAppActiveArgs {
    /// App name.
    #[arg(long)]
    pub app: Option<String>,

    /// Bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = 1500)]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 50)]
    pub poll_ms: u64,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(false)
            .args(["window_id", "active_window", "app"])
    )
)]
pub struct WaitWindowPresentArgs {
    /// Select by window id.
    #[arg(long)]
    pub window_id: Option<u32>,

    /// Select frontmost active window.
    #[arg(long)]
    pub active_window: bool,

    /// Select by app name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow app selection by window title substring.
    #[arg(
        long = "window-title-contains",
        visible_alias = "window-name",
        requires = "app"
    )]
    pub window_name: Option<String>,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = 1500)]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 50)]
    pub poll_ms: u64,
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use super::{
        AxActionCommand, AxAttrCommand, AxCommand, AxSessionCommand, AxWatchCommand, Cli,
        CommandGroup, ErrorFormat, InputSourceCommand, OutputFormat, WaitCommand, WindowCommand,
    };

    #[test]
    fn parses_window_activate_command_tree() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "--format",
            "json",
            "--retries",
            "2",
            "window",
            "activate",
            "--app",
            "Terminal",
            "--wait-ms",
            "1500",
        ])
        .expect("window activate should parse");

        assert_eq!(cli.format, OutputFormat::Json);
        assert_eq!(cli.error_format, ErrorFormat::Text);
        assert_eq!(cli.retries, 2);
        match cli.command {
            CommandGroup::Window {
                command: WindowCommand::Activate(args),
            } => {
                assert_eq!(args.app.as_deref(), Some("Terminal"));
                assert_eq!(args.wait_ms, Some(1500));
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_wait_window_present() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "wait",
            "window-present",
            "--app",
            "Terminal",
            "--window-title-contains",
            "Inbox",
            "--timeout-ms",
            "2000",
            "--poll-ms",
            "100",
        ])
        .expect("wait window-present should parse");

        match cli.command {
            CommandGroup::Wait {
                command: WaitCommand::WindowPresent(args),
            } => {
                assert_eq!(args.app.as_deref(), Some("Terminal"));
                assert_eq!(args.window_name.as_deref(), Some("Inbox"));
                assert_eq!(args.timeout_ms, 2000);
                assert_eq!(args.poll_ms, 100);
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_multiple_window_activate_selectors() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "window",
            "activate",
            "--window-id",
            "10",
            "--app",
            "Terminal",
        ])
        .expect_err("multiple selectors must be rejected");
        let rendered = err.to_string();
        assert!(
            rendered.contains("cannot be used with")
                || rendered.contains("required arguments were not provided")
        );
    }

    #[test]
    fn rejects_window_title_contains_without_app() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "wait",
            "window-present",
            "--window-title-contains",
            "Inbox",
        ])
        .expect_err("window-title-contains requires app");
        let rendered = err.to_string();
        assert!(
            rendered.contains("requires")
                || rendered.contains("required arguments were not provided")
        );
    }

    #[test]
    fn supports_window_name_alias_for_backward_compatibility() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "wait",
            "window-present",
            "--app",
            "Terminal",
            "--window-name",
            "Inbox",
        ])
        .expect("legacy --window-name alias should parse");

        match cli.command {
            CommandGroup::Wait {
                command: WaitCommand::WindowPresent(args),
            } => {
                assert_eq!(args.window_name.as_deref(), Some("Inbox"));
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_input_source_switch_command() {
        let cli = Cli::try_parse_from(["macos-agent", "input-source", "switch", "--id", "abc"])
            .expect("input-source switch should parse");

        match cli.command {
            CommandGroup::InputSource {
                command: InputSourceCommand::Switch(args),
            } => {
                assert_eq!(args.id, "abc".to_string());
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_input_type_submit_and_enter_alias() {
        let canonical = Cli::try_parse_from([
            "macos-agent",
            "input",
            "type",
            "--text",
            "hello",
            "--submit",
        ])
        .expect("input type --submit should parse");
        match canonical.command {
            CommandGroup::Input {
                command: super::InputCommand::Type(args),
            } => assert!(args.enter),
            other => panic!("unexpected command variant: {other:?}"),
        }

        let alias =
            Cli::try_parse_from(["macos-agent", "input", "type", "--text", "hello", "--enter"])
                .expect("input type --enter alias should parse");
        match alias.command {
            CommandGroup::Input {
                command: super::InputCommand::Type(args),
            } => assert!(args.enter),
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_ax_list_with_filters() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "list",
            "--app",
            "Arc",
            "--role",
            "AXButton",
            "--title-contains",
            "New tab",
            "--max-depth",
            "4",
            "--limit",
            "20",
        ])
        .expect("ax list should parse");

        match cli.command {
            CommandGroup::Ax {
                command: AxCommand::List(args),
            } => {
                assert_eq!(args.target.app.as_deref(), Some("Arc"));
                assert_eq!(args.filters.role.as_deref(), Some("AXButton"));
                assert_eq!(args.filters.title_contains.as_deref(), Some("New tab"));
                assert_eq!(args.max_depth, Some(4));
                assert_eq!(args.limit, Some(20));
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_ax_click_node_id_selector() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "--dry-run",
            "ax",
            "click",
            "--node-id",
            "node-17",
            "--allow-coordinate-fallback",
        ])
        .expect("ax click should parse");

        match cli.command {
            CommandGroup::Ax {
                command: AxCommand::Click(args),
            } => {
                assert_eq!(args.selector.node_id.as_deref(), Some("node-17"));
                assert!(args.allow_coordinate_fallback);
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_ax_type_compound_selector() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "type",
            "--role",
            "AXTextField",
            "--title-contains",
            "Search",
            "--nth",
            "2",
            "--text",
            "hello",
            "--clear-first",
            "--submit",
            "--paste",
            "--allow-keyboard-fallback",
        ])
        .expect("ax type should parse");

        match cli.command {
            CommandGroup::Ax {
                command: AxCommand::Type(args),
            } => {
                assert_eq!(args.selector.filters.role.as_deref(), Some("AXTextField"));
                assert_eq!(
                    args.selector.filters.title_contains.as_deref(),
                    Some("Search")
                );
                assert_eq!(args.selector.nth, Some(2));
                assert_eq!(args.text, "hello");
                assert!(args.clear_first);
                assert!(args.submit);
                assert!(args.paste);
                assert!(args.allow_keyboard_fallback);
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_ax_click_mixed_selectors() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "click",
            "--node-id",
            "node-17",
            "--role",
            "AXButton",
            "--title-contains",
            "Save",
        ])
        .expect_err("selector mix should be rejected");
        let rendered = err.to_string();
        assert!(
            rendered.contains("cannot be used with")
                || rendered.contains("required arguments were not provided")
        );
    }

    #[test]
    fn parses_ax_type_role_without_title_contains() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "type",
            "--role",
            "AXTextField",
            "--text",
            "hello",
        ])
        .expect("role-only selector should parse");
        match cli.command {
            CommandGroup::Ax {
                command: AxCommand::Type(args),
            } => {
                assert_eq!(args.selector.filters.role.as_deref(), Some("AXTextField"));
                assert!(args.selector.filters.title_contains.is_none());
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_ax_type_nth_without_selector_filter() {
        let err =
            Cli::try_parse_from(["macos-agent", "ax", "type", "--nth", "2", "--text", "hello"])
                .expect_err("nth alone should be rejected by selector group");
        let rendered = err.to_string();
        assert!(rendered.contains("required arguments were not provided"));
    }

    #[test]
    fn rejects_ax_list_multiple_target_selectors() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "list",
            "--app",
            "Arc",
            "--bundle-id",
            "com.apple.Safari",
        ])
        .expect_err("app and bundle-id should be mutually exclusive");
        let rendered = err.to_string();
        assert!(rendered.contains("cannot be used with"));
    }

    #[test]
    fn parses_ax_attr_get_and_set_commands() {
        let get_cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "attr",
            "get",
            "--node-id",
            "1.2",
            "--name",
            "AXRole",
        ])
        .expect("ax attr get should parse");
        match get_cli.command {
            CommandGroup::Ax {
                command:
                    AxCommand::Attr {
                        command: AxAttrCommand::Get(args),
                    },
            } => {
                assert_eq!(args.selector.node_id.as_deref(), Some("1.2"));
                assert_eq!(args.name, "AXRole");
            }
            other => panic!("unexpected command variant: {other:?}"),
        }

        let set_cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "attr",
            "set",
            "--role",
            "AXTextField",
            "--title-contains",
            "Search",
            "--name",
            "AXValue",
            "--value",
            "hello",
            "--value-type",
            "string",
        ])
        .expect("ax attr set should parse");
        match set_cli.command {
            CommandGroup::Ax {
                command:
                    AxCommand::Attr {
                        command: AxAttrCommand::Set(args),
                    },
            } => {
                assert_eq!(args.selector.filters.role.as_deref(), Some("AXTextField"));
                assert_eq!(
                    args.selector.filters.title_contains.as_deref(),
                    Some("Search")
                );
                assert_eq!(args.name, "AXValue");
                assert_eq!(args.value, "hello");
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }

    #[test]
    fn parses_ax_action_session_and_watch_commands() {
        let action_cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "action",
            "perform",
            "--node-id",
            "1.1",
            "--name",
            "AXPress",
        ])
        .expect("ax action perform should parse");
        match action_cli.command {
            CommandGroup::Ax {
                command:
                    AxCommand::Action {
                        command: AxActionCommand::Perform(args),
                    },
            } => {
                assert_eq!(args.selector.node_id.as_deref(), Some("1.1"));
                assert_eq!(args.name, "AXPress");
            }
            other => panic!("unexpected command variant: {other:?}"),
        }

        let session_cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "session",
            "start",
            "--app",
            "Arc",
            "--session-id",
            "axs-demo",
        ])
        .expect("ax session start should parse");
        match session_cli.command {
            CommandGroup::Ax {
                command:
                    AxCommand::Session {
                        command: AxSessionCommand::Start(args),
                    },
            } => {
                assert_eq!(args.app.as_deref(), Some("Arc"));
                assert_eq!(args.session_id.as_deref(), Some("axs-demo"));
            }
            other => panic!("unexpected command variant: {other:?}"),
        }

        let watch_cli = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "watch",
            "start",
            "--session-id",
            "axs-demo",
            "--events",
            "AXTitleChanged,AXFocusedUIElementChanged",
        ])
        .expect("ax watch start should parse");
        match watch_cli.command {
            CommandGroup::Ax {
                command:
                    AxCommand::Watch {
                        command: AxWatchCommand::Start(args),
                    },
            } => {
                assert_eq!(args.session_id, "axs-demo");
                assert_eq!(args.events.len(), 2);
            }
            other => panic!("unexpected command variant: {other:?}"),
        }
    }
}
