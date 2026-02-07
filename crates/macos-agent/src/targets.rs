use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::{ImageFormat, ListWindowsArgs};
use crate::error::CliError;
use crate::model::{AppRow, WindowRow};
use crate::screen_record_adapter::{
    self, AppInfo, ScreenshotFormat, ShareableContent, WindowInfo, WindowSelection,
};
use crate::test_mode;

#[derive(Debug, Clone, Default)]
pub struct TargetSelector {
    pub window_id: Option<u32>,
    pub active_window: bool,
    pub app: Option<String>,
    pub window_name: Option<String>,
}

pub fn list_windows(args: &ListWindowsArgs) -> Result<Vec<WindowRow>, CliError> {
    let content = fetch_shareable_content()?;
    let mut windows = content.windows;

    if let Some(app) = args.app.as_deref() {
        windows.retain(|window| contains_case_insensitive(&window.owner_name, app));
    }
    if let Some(name) = args.window_name.as_deref() {
        windows.retain(|window| contains_case_insensitive(&window.title, name));
    }
    if args.on_screen_only {
        windows.retain(|window| window.on_screen);
    }

    windows.sort_by(|a, b| {
        a.owner_name
            .cmp(&b.owner_name)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(windows.iter().map(WindowRow::from).collect())
}

pub fn list_apps() -> Result<Vec<AppRow>, CliError> {
    let content = fetch_shareable_content()?;
    let mut unique: BTreeMap<(String, i32, String), AppInfo> = BTreeMap::new();
    for app in content.apps {
        unique.insert((app.name.clone(), app.pid, app.bundle_id.clone()), app);
    }

    Ok(unique
        .into_values()
        .map(|app| AppRow::from(&app))
        .collect::<Vec<_>>())
}

pub fn resolve_window(selector: &TargetSelector) -> Result<WindowInfo, CliError> {
    let content = fetch_shareable_content()?;
    let args = WindowSelection {
        window_id: selector.window_id,
        active_window: selector.active_window,
        app: selector.app.clone(),
        window_name: selector.window_name.clone(),
    };

    screen_record_adapter::resolve_window(&content.windows, &args)
}

pub fn window_present(selector: &TargetSelector) -> Result<bool, CliError> {
    let content = fetch_shareable_content()?;
    if let Some(window_id) = selector.window_id {
        return Ok(content.windows.iter().any(|window| window.id == window_id));
    }

    if selector.active_window {
        return Ok(content.windows.iter().any(|window| window.active));
    }

    if let Some(app) = selector.app.as_deref() {
        let mut windows = content
            .windows
            .iter()
            .filter(|window| contains_case_insensitive(&window.owner_name, app));

        if let Some(window_name) = selector.window_name.as_deref() {
            return Ok(windows.any(|window| contains_case_insensitive(&window.title, window_name)));
        }

        return Ok(windows.next().is_some());
    }

    Ok(false)
}

pub fn app_active_by_name(app_name: &str) -> Result<bool, CliError> {
    let content = fetch_shareable_content()?;
    Ok(content
        .windows
        .iter()
        .any(|window| window.active && contains_case_insensitive(&window.owner_name, app_name)))
}

pub fn app_active_by_bundle_id(bundle_id: &str) -> Result<bool, CliError> {
    let content = fetch_shareable_content()?;

    let app_name = content
        .apps
        .iter()
        .find(|app| app.bundle_id.eq_ignore_ascii_case(bundle_id))
        .map(|app| app.name.clone());

    match app_name {
        Some(name) => app_active_by_name(&name),
        None => Ok(false),
    }
}

pub fn capture_screenshot(
    path: &Path,
    window: &WindowInfo,
    format: ImageFormat,
) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            CliError::runtime(format!("failed to create output directory: {err}"))
        })?;
    }

    let format = to_screenshot_format(format);

    if test_mode::enabled() {
        return screen_record_adapter::test_screenshot_fixture(path, format);
    }

    screen_record_adapter::capture_window_screenshot_macos(window, path, format)
}

pub fn extension_format(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpg),
        "webp" => Some(ImageFormat::Webp),
        _ => None,
    }
}

fn to_screenshot_format(format: ImageFormat) -> ScreenshotFormat {
    match format {
        ImageFormat::Png => ScreenshotFormat::Png,
        ImageFormat::Jpg => ScreenshotFormat::Jpg,
        ImageFormat::Webp => ScreenshotFormat::Webp,
    }
}

fn fetch_shareable_content() -> Result<ShareableContent, CliError> {
    if test_mode::enabled() {
        Ok(screen_record_adapter::test_shareable_content())
    } else {
        screen_record_adapter::fetch_shareable_macos()
    }
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nils_test_support::{EnvGuard, GlobalStateLock};
    use tempfile::TempDir;

    use super::{
        app_active_by_bundle_id, capture_screenshot, extension_format, list_apps, list_windows,
        resolve_window, window_present, TargetSelector,
    };
    use crate::cli::{ImageFormat, ListWindowsArgs};

    #[test]
    fn extension_format_supports_expected_values() {
        assert_eq!(
            extension_format(&PathBuf::from("a.png")),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            extension_format(&PathBuf::from("a.jpeg")),
            Some(ImageFormat::Jpg)
        );
        assert_eq!(
            extension_format(&PathBuf::from("a.webp")),
            Some(ImageFormat::Webp)
        );
        assert_eq!(extension_format(&PathBuf::from("a.txt")), None);
    }

    #[test]
    fn list_windows_is_sorted_and_filtered_in_test_mode() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        let rows = list_windows(&ListWindowsArgs {
            app: Some("Terminal".to_string()),
            window_name: None,
            on_screen_only: true,
        })
        .expect("list windows");

        let ids = rows.iter().map(|row| row.window_id).collect::<Vec<_>>();
        assert_eq!(ids, vec![101, 100]);
    }

    #[test]
    fn resolve_window_by_window_id() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        let target = resolve_window(&TargetSelector {
            window_id: Some(100),
            ..TargetSelector::default()
        })
        .expect("resolve by id");

        assert_eq!(target.id, 100);
    }

    #[test]
    fn list_apps_is_deterministic() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        let rows = list_apps().expect("list apps");
        let names = rows
            .iter()
            .map(|row| row.app_name.clone())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Finder".to_string(), "Terminal".to_string()]);
    }

    #[test]
    fn window_present_and_app_activity_cover_selector_variants() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        assert!(window_present(&TargetSelector {
            window_id: Some(100),
            ..TargetSelector::default()
        })
        .expect("window id exists"));

        assert!(window_present(&TargetSelector {
            active_window: true,
            ..TargetSelector::default()
        })
        .expect("active window exists"));

        assert!(window_present(&TargetSelector {
            app: Some("Terminal".to_string()),
            window_name: Some("Docs".to_string()),
            ..TargetSelector::default()
        })
        .expect("app/window selector exists"));

        assert!(!window_present(&TargetSelector {
            app: Some("Safari".to_string()),
            ..TargetSelector::default()
        })
        .expect("missing app should be false"));

        assert!(app_active_by_bundle_id("com.apple.Terminal").expect("bundle exists"));
        assert!(!app_active_by_bundle_id("com.example.missing").expect("bundle missing"));
    }

    #[test]
    fn capture_screenshot_uses_test_fixture_in_test_mode() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        let target = resolve_window(&TargetSelector {
            window_id: Some(100),
            ..TargetSelector::default()
        })
        .expect("resolve target");

        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("capture.png");
        capture_screenshot(&path, &target, ImageFormat::Png).expect("capture screenshot");
        assert!(path.is_file(), "screenshot file should exist");
        assert!(std::fs::metadata(&path).expect("metadata").len() > 0);
    }

    #[test]
    fn screen_record_error_mapping_preserves_usage_and_runtime() {
        let usage = screen_record::error::CliError::usage("bad selector");
        let runtime = screen_record::error::CliError::runtime("capture failed");

        let mapped_usage = crate::screen_record_adapter::map_error(usage);
        assert_eq!(mapped_usage.exit_code(), 2);
        assert!(mapped_usage.to_string().contains("bad selector"));

        let mapped_runtime = crate::screen_record_adapter::map_error(runtime);
        assert_eq!(mapped_runtime.exit_code(), 1);
        assert!(mapped_runtime.to_string().contains("capture failed"));
    }
}
