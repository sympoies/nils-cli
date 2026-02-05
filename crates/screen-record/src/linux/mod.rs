use std::path::Path;

use crate::cli::{AudioMode, ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::types::{ShareableContent, WindowInfo};

pub mod preflight;

pub fn shareable_content() -> Result<ShareableContent, CliError> {
    Err(CliError::runtime(
        "Linux X11 backend is not implemented yet",
    ))
}

pub fn screenshot_window(
    _window: &WindowInfo,
    _path: &Path,
    _format: ImageFormat,
) -> Result<(), CliError> {
    Err(CliError::runtime(
        "Linux X11 backend is not implemented yet",
    ))
}

pub fn record_window(
    _window: &WindowInfo,
    _duration: u64,
    _audio: AudioMode,
    _path: &Path,
    _format: ContainerFormat,
) -> Result<(), CliError> {
    Err(CliError::runtime(
        "Linux X11 backend is not implemented yet",
    ))
}

pub fn record_display(
    _display_id: u32,
    _duration: u64,
    _audio: AudioMode,
    _path: &Path,
    _format: ContainerFormat,
) -> Result<(), CliError> {
    Err(CliError::runtime(
        "Linux X11 backend is not implemented yet",
    ))
}

pub fn record_main_display(
    _duration: u64,
    _audio: AudioMode,
    _path: &Path,
    _format: ContainerFormat,
) -> Result<(), CliError> {
    Err(CliError::runtime(
        "Linux X11 backend is not implemented yet",
    ))
}
