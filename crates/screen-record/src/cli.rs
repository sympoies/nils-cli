use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "screen-record",
    version,
    about = "Record a single window on macOS."
)]
pub struct Cli {
    /// Print selectable windows as TSV and exit.
    #[arg(long)]
    pub list_windows: bool,

    /// Print selectable apps as TSV and exit.
    #[arg(long)]
    pub list_apps: bool,

    /// Check Screen Recording permission and exit.
    #[arg(long)]
    pub preflight: bool,

    /// Best-effort permission request and exit.
    #[arg(long)]
    pub request_permission: bool,

    /// Record a specific window id.
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
