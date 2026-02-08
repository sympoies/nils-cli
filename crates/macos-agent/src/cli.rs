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
    #[arg(long, requires = "app")]
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
    #[arg(long, requires = "app")]
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
pub struct AxListArgs {
    /// Select target app by name.
    #[arg(long)]
    pub app: Option<String>,

    /// Select target app by bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Filter by AX role.
    #[arg(long)]
    pub role: Option<String>,

    /// Filter by title substring.
    #[arg(long)]
    pub title_contains: Option<String>,

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
            .multiple(false)
            .args(["node_id", "role"])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["app", "bundle_id"])
    )
)]
pub struct AxClickArgs {
    /// Select by node id from `ax list`.
    #[arg(long)]
    pub node_id: Option<String>,

    /// Select by AX role (requires --title-contains).
    #[arg(long, requires = "title_contains")]
    pub role: Option<String>,

    /// Select by title substring (requires --role).
    #[arg(long, requires = "role")]
    pub title_contains: Option<String>,

    /// Select the nth match from compound selector results.
    #[arg(long, requires = "role")]
    pub nth: Option<u32>,

    /// Narrow selector to app name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow selector to bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// Allow coordinate fallback when AX press is unavailable.
    #[arg(long)]
    pub allow_coordinate_fallback: bool,
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(false)
            .args(["node_id", "role"])
    ),
    group(
        ArgGroup::new("target")
            .required(false)
            .multiple(false)
            .args(["app", "bundle_id"])
    )
)]
pub struct AxTypeArgs {
    /// Select by node id from `ax list`.
    #[arg(long)]
    pub node_id: Option<String>,

    /// Select by AX role (requires --title-contains).
    #[arg(long, requires = "title_contains")]
    pub role: Option<String>,

    /// Select by title substring (requires --role).
    #[arg(long, requires = "role")]
    pub title_contains: Option<String>,

    /// Select the nth match from compound selector results.
    #[arg(long, requires = "role")]
    pub nth: Option<u32>,

    /// Narrow selector to app name.
    #[arg(long)]
    pub app: Option<String>,

    /// Narrow selector to bundle id.
    #[arg(long)]
    pub bundle_id: Option<String>,

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
    #[arg(long)]
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
    #[arg(long, requires = "app")]
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
    #[arg(long, requires = "app")]
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
        AxCommand, Cli, CommandGroup, ErrorFormat, InputSourceCommand, OutputFormat, WaitCommand,
        WindowCommand,
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
            "--window-name",
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
    fn rejects_window_name_without_app() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "wait",
            "window-present",
            "--window-name",
            "Inbox",
        ])
        .expect_err("window-name requires app");
        let rendered = err.to_string();
        assert!(
            rendered.contains("requires")
                || rendered.contains("required arguments were not provided")
        );
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
                assert_eq!(args.app.as_deref(), Some("Arc"));
                assert_eq!(args.role.as_deref(), Some("AXButton"));
                assert_eq!(args.title_contains.as_deref(), Some("New tab"));
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
                assert_eq!(args.node_id.as_deref(), Some("node-17"));
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
                assert_eq!(args.role.as_deref(), Some("AXTextField"));
                assert_eq!(args.title_contains.as_deref(), Some("Search"));
                assert_eq!(args.nth, Some(2));
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
    fn rejects_ax_type_role_without_title_contains() {
        let err = Cli::try_parse_from([
            "macos-agent",
            "ax",
            "type",
            "--role",
            "AXTextField",
            "--text",
            "hello",
        ])
        .expect_err("role requires title-contains");
        let rendered = err.to_string();
        assert!(
            rendered.contains("requires")
                || rendered.contains("required arguments were not provided")
        );
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
}
