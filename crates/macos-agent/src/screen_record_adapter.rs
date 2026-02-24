use std::path::Path;

use screen_record::select::{SelectionArgs, select_window};

use crate::error::CliError;

pub(crate) use screen_record::types::{AppInfo, ShareableContent, WindowInfo};

#[derive(Debug, Clone, Copy)]
pub struct ImageCropRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

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

pub fn crop_image(input: &Path, output: &Path, region: ImageCropRegion) -> Result<(), CliError> {
    let image = image::open(input).map_err(|err| {
        CliError::runtime(format!("failed to decode screenshot for cropping: {err}"))
    })?;

    let max_width = image.width();
    let max_height = image.height();
    if max_width == 0 || max_height == 0 {
        return Err(CliError::runtime("cannot crop empty screenshot image"));
    }

    let x = region.x.min(max_width.saturating_sub(1));
    let y = region.y.min(max_height.saturating_sub(1));
    let width = region.width.max(1).min(max_width.saturating_sub(x));
    let height = region.height.max(1).min(max_height.saturating_sub(y));

    let cropped = image.crop_imm(x, y, width, height);
    cropped
        .save(output)
        .map_err(|err| CliError::runtime(format!("failed to write cropped screenshot: {err}")))
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
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::{
        ImageCropRegion, ScreenshotFormat, crop_image, map_error, test_screenshot_fixture,
    };

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

    #[test]
    fn crop_image_writes_bounded_output() {
        let temp = TempDir::new().expect("tempdir");
        let input = temp.path().join("in.png");
        let output = temp.path().join("out.png");

        let image = image::DynamicImage::new_rgba8(100, 80);
        image.save(&input).expect("save input");

        crop_image(
            &input,
            &output,
            ImageCropRegion {
                x: 10,
                y: 12,
                width: 20,
                height: 16,
            },
        )
        .expect("crop should succeed");

        let cropped = image::open(&output).expect("open output");
        assert_eq!(cropped.width(), 20);
        assert_eq!(cropped.height(), 16);
    }

    #[test]
    fn crop_image_reports_decode_error_for_non_image_input() {
        let temp = TempDir::new().expect("tempdir");
        let input = temp.path().join("not-image.txt");
        let output = temp.path().join("out.png");
        fs::write(&input, "not an image").expect("write invalid image payload");

        let err = crop_image(
            &input,
            &output,
            ImageCropRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        )
        .expect_err("invalid image should fail decode");
        assert!(
            err.to_string()
                .contains("failed to decode screenshot for cropping")
        );
    }

    #[test]
    fn test_screenshot_fixture_supports_jpg_and_webp_formats() {
        let temp = TempDir::new().expect("tempdir");
        let jpg = temp.path().join("shot.jpg");
        let webp = temp.path().join("shot.webp");

        test_screenshot_fixture(&jpg, ScreenshotFormat::Jpg).expect("jpg screenshot fixture");
        test_screenshot_fixture(&webp, ScreenshotFormat::Webp).expect("webp screenshot fixture");

        assert!(jpg.exists());
        assert!(webp.exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn macos_only_screen_record_functions_return_unsupported_on_non_macos() {
        use nils_test_support::{EnvGuard, GlobalStateLock};

        use super::{
            capture_window_screenshot_macos, fetch_shareable_macos, test_shareable_content,
        };

        let lock = GlobalStateLock::new();
        let _test_mode = EnvGuard::remove(&lock, "AGENTS_MACOS_AGENT_TEST_MODE");

        let err = fetch_shareable_macos().expect_err("non-macos should be unsupported");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().to_ascii_lowercase().contains("unsupported"));

        let temp = TempDir::new().expect("tempdir");
        let out = temp.path().join("shot.png");
        let window = test_shareable_content()
            .windows
            .into_iter()
            .next()
            .expect("test window");
        let err = capture_window_screenshot_macos(&window, &out, ScreenshotFormat::Png)
            .expect_err("non-macos screenshot should be unsupported");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().to_ascii_lowercase().contains("unsupported"));
    }
}
