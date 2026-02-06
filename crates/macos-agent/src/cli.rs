use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Tsv,
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

    /// Print actions without mutating desktop state when supported.
    #[arg(long, global = true)]
    pub dry_run: bool,

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

    /// Capture screenshots for observation.
    Observe {
        #[command(subcommand)]
        command: ObserveCommand,
    },
}

#[derive(Debug, Clone, Args)]
pub struct PreflightArgs {
    /// Enable stricter checks in future preflight behavior.
    #[arg(long)]
    pub strict: bool,
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

    /// Wait this many milliseconds after activation.
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

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use super::{Cli, CommandGroup, OutputFormat, WindowCommand};

    #[test]
    fn parses_window_activate_command_tree() {
        let cli = Cli::try_parse_from([
            "macos-agent",
            "--format",
            "json",
            "window",
            "activate",
            "--app",
            "Terminal",
            "--wait-ms",
            "1500",
        ])
        .expect("window activate should parse");

        assert_eq!(cli.format, OutputFormat::Json);
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
}
