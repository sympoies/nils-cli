use std::path::Path;

use crate::cli::{AudioMode, ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::types::{ShareableContent, WindowInfo};

pub(crate) mod audio;
pub(crate) mod ffmpeg;
pub mod preflight;
#[cfg(target_os = "linux")]
pub(crate) mod x11;

pub fn shareable_content() -> Result<ShareableContent, CliError> {
    x11::fetch_shareable_content()
}

pub fn screenshot_window(
    window: &WindowInfo,
    path: &Path,
    format: ImageFormat,
) -> Result<(), CliError> {
    preflight::preflight()?;
    ffmpeg::screenshot_window(window, path, format)
}

pub fn record_window(
    window: &WindowInfo,
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    preflight::preflight()?;
    ffmpeg::record_window(window, duration, audio, path, format)
}

pub fn record_display(
    display_id: u32,
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    preflight::preflight()?;
    ffmpeg::record_display(display_id, duration, audio, path, format)
}

pub fn record_main_display(
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    preflight::preflight()?;
    ffmpeg::record_main_display(duration, audio, path, format)
}
