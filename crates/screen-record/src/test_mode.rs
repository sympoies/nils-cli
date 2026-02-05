use std::path::{Path, PathBuf};

use crate::cli::{ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::types::{AppInfo, Rect, ShareableContent, WindowInfo};

pub fn enabled() -> bool {
    let Some(value) = std::env::var_os("CODEX_SCREEN_RECORD_TEST_MODE") else {
        return false;
    };
    let value = value.to_string_lossy();
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

pub fn shareable_content() -> ShareableContent {
    let windows = vec![
        WindowInfo {
            id: 100,
            owner_name: "Terminal".to_string(),
            title: "Inbox".to_string(),
            bounds: Rect {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            on_screen: true,
            active: true,
            owner_pid: 111,
            z_order: 0,
        },
        WindowInfo {
            id: 101,
            owner_name: "Terminal".to_string(),
            title: "Docs".to_string(),
            bounds: Rect {
                x: 40,
                y: 40,
                width: 1100,
                height: 760,
            },
            on_screen: true,
            active: false,
            owner_pid: 111,
            z_order: 1,
        },
        WindowInfo {
            id: 200,
            owner_name: "Finder".to_string(),
            title: "Finder".to_string(),
            bounds: Rect {
                x: 80,
                y: 80,
                width: 900,
                height: 600,
            },
            on_screen: true,
            active: false,
            owner_pid: 222,
            z_order: 2,
        },
    ];

    let apps = vec![
        AppInfo {
            name: "Terminal".to_string(),
            pid: 111,
            bundle_id: "com.apple.Terminal".to_string(),
        },
        AppInfo {
            name: "Finder".to_string(),
            pid: 222,
            bundle_id: "com.apple.Finder".to_string(),
        },
    ];

    ShareableContent { windows, apps }
}

pub fn record_fixture(path: &Path, format: ContainerFormat) -> Result<(), CliError> {
    let source = fixture_path(format);
    if !source.exists() {
        return Err(CliError::runtime(format!(
            "fixture not found: {}",
            source.display()
        )));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;
    }

    std::fs::copy(&source, path)
        .map_err(|err| CliError::runtime(format!("failed to write output: {err}")))?;
    Ok(())
}

pub fn screenshot_fixture(path: &Path, format: ImageFormat) -> Result<(), CliError> {
    let source = screenshot_fixture_path(format);
    if !source.exists() {
        return Err(CliError::runtime(format!(
            "fixture not found: {}",
            source.display()
        )));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;
    }

    std::fs::copy(&source, path)
        .map_err(|err| CliError::runtime(format!("failed to write output: {err}")))?;
    Ok(())
}

pub struct TestWriter {
    path: PathBuf,
    format: ContainerFormat,
    appended: bool,
}

impl TestWriter {
    pub fn new(path: &Path, format: ContainerFormat) -> Self {
        Self {
            path: path.to_path_buf(),
            format,
            appended: false,
        }
    }

    pub fn append_frame(&mut self, _data: &[u8]) -> Result<(), CliError> {
        self.appended = true;
        Ok(())
    }

    pub fn finish(self) -> Result<(), CliError> {
        if !self.appended {
            return Err(CliError::runtime("no frames appended"));
        }
        record_fixture(&self.path, self.format)
    }
}

fn fixture_path(format: ContainerFormat) -> PathBuf {
    let filename = match format {
        ContainerFormat::Mov => "sample.mov",
        ContainerFormat::Mp4 => "sample.mp4",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(filename)
}

fn screenshot_fixture_path(format: ImageFormat) -> PathBuf {
    let filename = match format {
        ImageFormat::Png => "sample.png",
        ImageFormat::Jpg => "sample.jpg",
        ImageFormat::Webp => "sample.webp",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(filename)
}
