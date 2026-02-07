use std::path::Path;

use screen_record::select::{select_window, SelectionArgs};

use crate::error::CliError;

pub(crate) use screen_record::types::{AppInfo, ShareableContent, WindowInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    Png,
    Jpg,
    Webp,
}

#[derive(Debug, Clone, Default)]
pub struct WindowSelection {
    pub window_id: Option<u32>,
    pub active_window: bool,
    pub app: Option<String>,
    pub window_name: Option<String>,
}

pub fn resolve_window(
    windows: &[WindowInfo],
    selection: &WindowSelection,
) -> Result<WindowInfo, CliError> {
    let args = SelectionArgs {
        window_id: selection.window_id,
        app: selection.app.clone(),
        window_name: selection.window_name.clone(),
        active_window: selection.active_window,
    };

    select_window(windows, &args).map_err(map_error)
}

pub fn fetch_shareable_macos() -> Result<ShareableContent, CliError> {
    #[cfg(target_os = "macos")]
    {
        screen_record::macos::shareable::fetch_shareable().map_err(map_error)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(CliError::unsupported_platform())
    }
}

pub fn capture_window_screenshot_macos(
    window: &WindowInfo,
    path: &Path,
    format: ScreenshotFormat,
) -> Result<(), CliError> {
    #[cfg(target_os = "macos")]
    {
        screen_record::macos::screenshot::screenshot_window(
            window,
            path,
            to_screen_record_format(format),
        )
        .map_err(map_error)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        let _ = path;
        let _ = format;
        Err(CliError::unsupported_platform())
    }
}

pub fn test_shareable_content() -> ShareableContent {
    screen_record::test_mode::shareable_content()
}

pub fn test_screenshot_fixture(path: &Path, format: ScreenshotFormat) -> Result<(), CliError> {
    screen_record::test_mode::screenshot_fixture(path, to_screen_record_format(format))
        .map_err(map_error)
}

pub fn map_error(err: screen_record::error::CliError) -> CliError {
    if err.exit_code() == 2 {
        CliError::usage(err.to_string())
    } else {
        CliError::runtime(err.to_string())
    }
}

fn to_screen_record_format(format: ScreenshotFormat) -> screen_record::cli::ImageFormat {
    match format {
        ScreenshotFormat::Png => screen_record::cli::ImageFormat::Png,
        ScreenshotFormat::Jpg => screen_record::cli::ImageFormat::Jpg,
        ScreenshotFormat::Webp => screen_record::cli::ImageFormat::Webp,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::map_error;

    #[test]
    fn map_error_preserves_usage_and_runtime_exit_code() {
        let usage = screen_record::error::CliError::usage("bad selector");
        let runtime = screen_record::error::CliError::runtime("capture failed");

        let mapped_usage = map_error(usage);
        assert_eq!(mapped_usage.exit_code(), 2);
        assert!(mapped_usage.to_string().contains("bad selector"));

        let mapped_runtime = map_error(runtime);
        assert_eq!(mapped_runtime.exit_code(), 1);
        assert!(mapped_runtime.to_string().contains("capture failed"));
    }
}
