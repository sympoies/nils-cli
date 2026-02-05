use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "screen-record",
    version,
    about = "Record a single window or display on macOS (12+) or Linux (X11/Wayland portal)."
)]
pub struct Cli {
    /// Capture a single window screenshot and exit.
    #[arg(long)]
    pub screenshot: bool,

    /// Use the system portal picker (Linux Wayland) instead of X11 selectors.
    #[arg(long)]
    pub portal: bool,

    /// Print selectable windows as TSV and exit.
    #[arg(long)]
    pub list_windows: bool,

    /// Print selectable displays as TSV and exit.
    #[arg(long)]
    pub list_displays: bool,

    /// Print selectable apps as TSV and exit.
    #[arg(long)]
    pub list_apps: bool,

    /// Check capture prerequisites (macOS permission or X11/ffmpeg availability) and exit.
    #[arg(long)]
    pub preflight: bool,

    /// Best-effort permission request (macOS) or prerequisite check (X11), then exit.
    #[arg(long)]
    pub request_permission: bool,

    /// Record a specific window id (from --list-windows).
    #[arg(long, value_name = "id")]
    pub window_id: Option<u32>,

    /// Select a window by app/owner name (case-insensitive substring).
    #[arg(long, value_name = "name")]
    pub app: Option<String>,

    /// Narrow --app selection by window title substring.
    #[arg(long, value_name = "name", requires = "app")]
    pub window_name: Option<String>,

    /// Record the frontmost window on the current Space.
    #[arg(long)]
    pub active_window: bool,

    /// Record the primary display.
    #[arg(long)]
    pub display: bool,

    /// Record a specific display id (from --list-displays).
    #[arg(long, value_name = "id")]
    pub display_id: Option<u32>,

    /// Record for N seconds.
    #[arg(long, value_name = "seconds")]
    pub duration: Option<u64>,

    /// Control audio capture.
    #[arg(long, value_enum, default_value_t = AudioMode::Off)]
    pub audio: AudioMode,

    /// Output file path.
    #[arg(long, value_name = "path")]
    pub path: Option<PathBuf>,

    /// Explicit container selection. Overrides extension.
    #[arg(long, value_enum)]
    pub format: Option<ContainerFormat>,

    /// Screenshot output image format. Overrides extension.
    #[arg(long, value_enum)]
    pub image_format: Option<ImageFormat>,

    /// Output directory for screenshot mode when --path is omitted.
    #[arg(long, value_name = "path")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AudioMode {
    Off,
    System,
    Mic,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ContainerFormat {
    Mov,
    Mp4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImageFormat {
    Png,
    #[value(alias = "jpeg")]
    Jpg,
    Webp,
}
