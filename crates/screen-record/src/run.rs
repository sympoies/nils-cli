use std::path::{Path, PathBuf};

use chrono::Local;

use crate::cli::{AudioMode, Cli, ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::select::{format_window_tsv, select_window, SelectionArgs};
use crate::test_mode;
use crate::types::{AppInfo, DisplayInfo, ShareableContent, WindowInfo};

pub fn run(cli: Cli) -> Result<(), CliError> {
    let test_mode_enabled = test_mode::enabled();
    let mode = determine_mode(&cli)?;
    validate_screenshot_flag_usage(&cli, mode)?;
    validate_portal_flag_usage(&cli, mode)?;
    let backend = Backend::detect(test_mode_enabled)?;

    match mode {
        Mode::Preflight => {
            ensure_no_recording_flags(&cli)?;
            backend.preflight()?;
            return Ok(());
        }
        Mode::RequestPermission => {
            ensure_no_recording_flags(&cli)?;
            backend.request_permission()?;
            return Ok(());
        }
        Mode::ListWindows => {
            ensure_no_recording_flags(&cli)?;
            if matches!(backend, Backend::Linux) {
                ensure_linux_x11_only_mode_allowed()?;
            }
            let content = fetch_shareable_content(&backend)?;
            let mut windows = content.windows;
            windows.sort_by(|a, b| {
                a.owner_name
                    .cmp(&b.owner_name)
                    .then_with(|| a.title.cmp(&b.title))
                    .then_with(|| a.id.cmp(&b.id))
            });
            for window in windows {
                println!("{}", format_window_tsv(&window));
            }
        }
        Mode::ListDisplays => {
            ensure_no_recording_flags(&cli)?;
            if matches!(backend, Backend::Linux) {
                ensure_linux_x11_only_mode_allowed()?;
            }
            let content = fetch_shareable_content(&backend)?;
            let mut displays = content.displays;
            displays.sort_by(|a, b| a.id.cmp(&b.id));
            for display in displays {
                println!("{}", format_display_tsv(&display));
            }
        }
        Mode::ListApps => {
            ensure_no_recording_flags(&cli)?;
            if matches!(backend, Backend::Linux) {
                ensure_linux_x11_only_mode_allowed()?;
            }
            let content = fetch_shareable_content(&backend)?;
            let mut apps = content.apps;
            normalize_app_list(&mut apps);
            for app in apps {
                println!("{}", format_app_tsv(&app));
            }
        }
        Mode::Screenshot => {
            validate_screenshot_args(&cli)?;
            if cli.portal {
                let (output_path, format) =
                    resolve_portal_screenshot_output(&cli, test_mode_enabled)?;
                backend.screenshot_portal(&output_path, format)?;
                println!("{}", output_path.display());
                return Ok(());
            }

            if matches!(backend, Backend::Linux) {
                ensure_linux_x11_selectors_allowed()?;
            }

            let content = fetch_shareable_content(&backend)?;
            let args = SelectionArgs {
                window_id: cli.window_id,
                app: cli.app.clone(),
                window_name: cli.window_name.clone(),
                active_window: cli.active_window,
            };
            let selected = select_window(&content.windows, &args)?;

            let (output_path, format) =
                resolve_screenshot_output(&cli, &selected, test_mode_enabled)?;
            backend.screenshot_window(&selected, &output_path, format)?;
            println!("{}", output_path.display());
            return Ok(());
        }
        Mode::Record => {
            validate_record_args(&cli)?;

            let output_path = resolve_output_path(&cli)?;
            let container = resolve_container(&output_path, cli.format)?;
            if cli.audio == AudioMode::Both && container == ContainerFormat::Mp4 {
                return Err(CliError::usage("--audio both requires .mov"));
            }
            if cli.portal {
                backend.record_portal(
                    cli.duration.expect("duration validated"),
                    &output_path,
                    container,
                )?;
            } else if matches!(backend, Backend::Linux) {
                ensure_linux_x11_selectors_allowed()?;
                if cli.display {
                    backend.record_main_display(
                        cli.duration.expect("duration validated"),
                        cli.audio,
                        &output_path,
                        container,
                    )?;
                } else if let Some(display_id) = cli.display_id {
                    backend.record_display(
                        display_id,
                        cli.duration.expect("duration validated"),
                        cli.audio,
                        &output_path,
                        container,
                    )?;
                } else {
                    let content = fetch_shareable_content(&backend)?;
                    let args = SelectionArgs {
                        window_id: cli.window_id,
                        app: cli.app.clone(),
                        window_name: cli.window_name.clone(),
                        active_window: cli.active_window,
                    };
                    let selected = select_window(&content.windows, &args)?;
                    backend.record_window(
                        &selected,
                        cli.duration.expect("duration validated"),
                        cli.audio,
                        &output_path,
                        container,
                    )?;
                }
            } else if cli.display {
                backend.record_main_display(
                    cli.duration.expect("duration validated"),
                    cli.audio,
                    &output_path,
                    container,
                )?;
            } else if let Some(display_id) = cli.display_id {
                backend.record_display(
                    display_id,
                    cli.duration.expect("duration validated"),
                    cli.audio,
                    &output_path,
                    container,
                )?;
            } else {
                let content = fetch_shareable_content(&backend)?;
                let args = SelectionArgs {
                    window_id: cli.window_id,
                    app: cli.app.clone(),
                    window_name: cli.window_name.clone(),
                    active_window: cli.active_window,
                };
                let selected = select_window(&content.windows, &args)?;
                backend.record_window(
                    &selected,
                    cli.duration.expect("duration validated"),
                    cli.audio,
                    &output_path,
                    container,
                )?;
            }
            println!("{}", output_path.display());
            return Ok(());
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ListWindows,
    ListDisplays,
    ListApps,
    Preflight,
    RequestPermission,
    Screenshot,
    Record,
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    TestMode,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Macos,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Linux,
}

impl Backend {
    fn detect(test_mode_enabled: bool) -> Result<Self, CliError> {
        if test_mode_enabled {
            return Ok(Backend::TestMode);
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Backend::Macos)
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Backend::Linux)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(CliError::unsupported_platform())
        }
    }

    fn preflight(&self) -> Result<(), CliError> {
        match self {
            Backend::TestMode => Ok(()),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::permissions::preflight()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::preflight::preflight()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn request_permission(&self) -> Result<(), CliError> {
        match self {
            Backend::TestMode => Ok(()),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::permissions::request_permission()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::preflight::request_permission()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn shareable_content(&self) -> Result<ShareableContent, CliError> {
        match self {
            Backend::TestMode => Ok(test_mode::shareable_content()),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::shareable::fetch_shareable()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::shareable_content()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn screenshot_window(
        &self,
        window: &WindowInfo,
        path: &Path,
        format: ImageFormat,
    ) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::screenshot_fixture(path, format),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::screenshot::screenshot_window(window, path, format)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (window, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::screenshot_window(window, path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (window, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn screenshot_portal(&self, path: &Path, format: ImageFormat) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::screenshot_fixture(path, format),
            Backend::Macos => Err(CliError::usage(
                "--portal is only supported on Linux (Wayland)",
            )),
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::screenshot_portal(path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn record_window(
        &self,
        window: &WindowInfo,
        duration: u64,
        audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::record_fixture(path, format),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::stream::record_window(window, duration, audio, path, format)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (window, duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::record_window(window, duration, audio, path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (window, duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn record_display(
        &self,
        display_id: u32,
        duration: u64,
        audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::record_fixture(path, format),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::stream::record_display(display_id, duration, audio, path, format)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (display_id, duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::record_display(display_id, duration, audio, path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (display_id, duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn record_main_display(
        &self,
        duration: u64,
        audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::record_fixture(path, format),
            Backend::Macos => {
                #[cfg(target_os = "macos")]
                {
                    crate::macos::stream::record_main_display(duration, audio, path, format)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::record_main_display(duration, audio, path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (duration, audio, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }

    fn record_portal(
        &self,
        duration: u64,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        match self {
            Backend::TestMode => test_mode::record_fixture(path, format),
            Backend::Macos => Err(CliError::usage(
                "--portal is only supported on Linux (Wayland)",
            )),
            Backend::Linux => {
                #[cfg(target_os = "linux")]
                {
                    crate::linux::record_portal(duration, path, format)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (duration, path, format);
                    Err(CliError::unsupported_platform())
                }
            }
        }
    }
}

fn determine_mode(cli: &Cli) -> Result<Mode, CliError> {
    let mut modes = Vec::new();
    if cli.list_windows {
        modes.push(Mode::ListWindows);
    }
    if cli.list_displays {
        modes.push(Mode::ListDisplays);
    }
    if cli.list_apps {
        modes.push(Mode::ListApps);
    }
    if cli.preflight {
        modes.push(Mode::Preflight);
    }
    if cli.request_permission {
        modes.push(Mode::RequestPermission);
    }
    if cli.screenshot {
        modes.push(Mode::Screenshot);
    }

    if modes.len() > 1 {
        return Err(CliError::usage(
            "select exactly one mode: --list-windows, --list-displays, --list-apps, --preflight, --request-permission, or --screenshot",
        ));
    }

    Ok(modes.pop().unwrap_or(Mode::Record))
}

fn validate_screenshot_flag_usage(cli: &Cli, mode: Mode) -> Result<(), CliError> {
    if mode != Mode::Screenshot && (cli.image_format.is_some() || cli.dir.is_some()) {
        return Err(CliError::usage("screenshot flags require --screenshot"));
    }
    Ok(())
}

fn validate_portal_flag_usage(cli: &Cli, mode: Mode) -> Result<(), CliError> {
    if !cli.portal {
        return Ok(());
    }

    match mode {
        Mode::Record | Mode::Screenshot => {}
        _ => {
            return Err(CliError::usage(
                "--portal is only valid with recording or --screenshot",
            ));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = mode;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mode;
        Err(CliError::usage(
            "--portal is only supported on Linux (Wayland)",
        ))
    }
}

fn validate_record_args(cli: &Cli) -> Result<(), CliError> {
    if cli.image_format.is_some() || cli.dir.is_some() {
        return Err(CliError::usage("screenshot flags require --screenshot"));
    }

    let selection_count = cli.window_id.is_some() as u8
        + cli.active_window as u8
        + cli.app.is_some() as u8
        + cli.portal as u8
        + cli.display as u8
        + cli.display_id.is_some() as u8;

    if selection_count != 1 {
        return Err(CliError::usage(
            "recording requires exactly one selector: --portal, --window-id, --active-window, --app, --display, or --display-id",
        ));
    }

    if cli.window_name.is_some() && cli.app.is_none() {
        return Err(CliError::usage("--window-name requires --app"));
    }

    if (cli.display || cli.display_id.is_some()) && cli.window_name.is_some() {
        return Err(CliError::usage("--window-name is only valid with --app"));
    }

    if cli.portal && cli.audio != AudioMode::Off {
        return Err(CliError::usage(
            "--portal does not support audio; use --audio off",
        ));
    }

    if cli.duration.is_none() {
        return Err(CliError::usage("--duration is required for recording"));
    }

    if cli.path.is_none() {
        return Err(CliError::usage("--path is required for recording"));
    }

    Ok(())
}

fn validate_screenshot_args(cli: &Cli) -> Result<(), CliError> {
    if !cli.screenshot {
        return Err(CliError::usage("missing --screenshot"));
    }

    if cli.display || cli.display_id.is_some() {
        return Err(CliError::usage(
            "display selectors are not valid with --screenshot",
        ));
    }

    let selection_count = cli.portal as u8
        + cli.window_id.is_some() as u8
        + cli.active_window as u8
        + cli.app.is_some() as u8;

    if selection_count != 1 {
        return Err(CliError::usage(
            "screenshot requires exactly one selector: --portal, --window-id, --active-window, or --app",
        ));
    }

    if cli.window_name.is_some() && cli.app.is_none() {
        return Err(CliError::usage("--window-name requires --app"));
    }

    if cli.duration.is_some() {
        return Err(CliError::usage("--duration is not valid with --screenshot"));
    }

    if cli.audio != AudioMode::Off {
        return Err(CliError::usage("--audio is not valid with --screenshot"));
    }

    if cli.format.is_some() {
        return Err(CliError::usage("--format is not valid with --screenshot"));
    }

    if cli.path.is_some() && cli.dir.is_some() {
        return Err(CliError::usage("use either --path or --dir"));
    }

    Ok(())
}

fn ensure_no_recording_flags(cli: &Cli) -> Result<(), CliError> {
    if cli.portal
        || cli.window_id.is_some()
        || cli.app.is_some()
        || cli.window_name.is_some()
        || cli.active_window
        || cli.display
        || cli.display_id.is_some()
        || cli.duration.is_some()
        || cli.screenshot
        || cli.path.is_some()
        || cli.format.is_some()
        || cli.image_format.is_some()
        || cli.dir.is_some()
        || cli.audio != AudioMode::Off
    {
        return Err(CliError::usage(
            "capture flags are not valid with this mode",
        ));
    }
    Ok(())
}

fn resolve_output_path(cli: &Cli) -> Result<PathBuf, CliError> {
    let mut path = cli
        .path
        .clone()
        .ok_or_else(|| CliError::usage("--path is required for recording"))?;

    if !path.is_absolute() {
        let cwd = std::env::current_dir()
            .map_err(|err| CliError::runtime(format!("failed to resolve cwd: {err}")))?;
        path = cwd.join(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;
    }

    Ok(path)
}

fn resolve_portal_screenshot_output(
    cli: &Cli,
    test_mode_enabled: bool,
) -> Result<(PathBuf, ImageFormat), CliError> {
    let format = resolve_image_format(cli.path.as_deref(), cli.image_format)?;
    let cwd = std::env::current_dir()
        .map_err(|err| CliError::runtime(format!("failed to resolve cwd: {err}")))?;

    if let Some(path) = cli.path.as_ref() {
        let mut path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };

        if path.extension().is_none() {
            path.set_extension(image_ext(format));
        }

        if path.is_dir() {
            return Err(CliError::usage("--path must be a file path"));
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;
        }

        return Ok((path, format));
    }

    let dir = cli
        .dir
        .as_ref()
        .map(|dir| {
            if dir.is_absolute() {
                dir.clone()
            } else {
                cwd.join(dir)
            }
        })
        .unwrap_or_else(|| cwd.join("screenshots"));

    if dir.exists() && !dir.is_dir() {
        return Err(CliError::usage("--dir must be a directory"));
    }

    std::fs::create_dir_all(&dir)
        .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;

    let timestamp = screenshot_timestamp(test_mode_enabled);
    let stem = format!("screenshot-{timestamp}-portal");
    let ext = image_ext(format);

    let mut candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return Ok((candidate, format));
    }

    for idx in 2..=u32::MAX {
        candidate = dir.join(format!("{stem}-{idx}.{ext}"));
        if !candidate.exists() {
            return Ok((candidate, format));
        }
    }

    Err(CliError::runtime("failed to choose a unique output path"))
}

fn resolve_screenshot_output(
    cli: &Cli,
    window: &WindowInfo,
    test_mode_enabled: bool,
) -> Result<(PathBuf, ImageFormat), CliError> {
    let format = resolve_image_format(cli.path.as_deref(), cli.image_format)?;

    let cwd = std::env::current_dir()
        .map_err(|err| CliError::runtime(format!("failed to resolve cwd: {err}")))?;

    if let Some(path) = cli.path.as_ref() {
        let mut path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };

        if path.extension().is_none() {
            path.set_extension(image_ext(format));
        }

        if path.is_dir() {
            return Err(CliError::usage("--path must be a file path"));
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;
        }

        return Ok((path, format));
    }

    let dir = cli
        .dir
        .as_ref()
        .map(|dir| {
            if dir.is_absolute() {
                dir.clone()
            } else {
                cwd.join(dir)
            }
        })
        .unwrap_or_else(|| cwd.join("screenshots"));

    if dir.exists() && !dir.is_dir() {
        return Err(CliError::usage("--dir must be a directory"));
    }

    std::fs::create_dir_all(&dir)
        .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;

    let timestamp = screenshot_timestamp(test_mode_enabled);
    let stem = default_screenshot_stem(&timestamp, window);
    let ext = image_ext(format);

    let mut candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return Ok((candidate, format));
    }

    for idx in 2..=u32::MAX {
        candidate = dir.join(format!("{stem}-{idx}.{ext}"));
        if !candidate.exists() {
            return Ok((candidate, format));
        }
    }

    Err(CliError::runtime("failed to choose a unique output path"))
}

fn resolve_image_format(
    path: Option<&Path>,
    format: Option<ImageFormat>,
) -> Result<ImageFormat, CliError> {
    let ext = path
        .and_then(|path| path.extension())
        .map(|value| value.to_string_lossy().to_ascii_lowercase());

    let ext_format = match ext.as_deref() {
        Some("png") => Some(ImageFormat::Png),
        Some("jpg") | Some("jpeg") => Some(ImageFormat::Jpg),
        Some("webp") => Some(ImageFormat::Webp),
        Some(_) => None,
        None => None,
    };

    if let Some(format) = format {
        if let Some(ext) = ext.as_deref() {
            if !matches!(ext, "png" | "jpg" | "jpeg" | "webp") {
                return Err(CliError::usage(
                    "unsupported --path extension for screenshot (supported: .png, .jpg, .jpeg, .webp)",
                ));
            }
        }

        if let Some(ext_format) = ext_format {
            if ext_format != format {
                return Err(CliError::usage(format!(
                    "--image-format {} conflicts with --path extension",
                    image_label(format)
                )));
            }
        }
        return Ok(format);
    }

    if ext.is_some() {
        return ext_format.ok_or_else(|| {
            CliError::usage(
                "unsupported --path extension for screenshot (supported: .png, .jpg, .jpeg, .webp)",
            )
        });
    }

    Ok(ImageFormat::Png)
}

fn image_label(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpg => "jpg",
        ImageFormat::Webp => "webp",
    }
}

fn image_ext(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpg => "jpg",
        ImageFormat::Webp => "webp",
    }
}

fn screenshot_timestamp(test_mode_enabled: bool) -> String {
    if test_mode_enabled {
        if let Some(value) = std::env::var_os("CODEX_SCREEN_RECORD_TEST_TIMESTAMP") {
            let value = value.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
        return "20260101-000000".to_string();
    }

    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn default_screenshot_stem(timestamp: &str, window: &WindowInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push("screenshot".to_string());
    parts.push(timestamp.to_string());
    parts.push(format!("win{}", window.id));

    if let Some(owner) = sanitize_filename_segment(&window.owner_name) {
        parts.push(owner);
    }
    if let Some(title) = sanitize_filename_segment(&window.title) {
        parts.push(title);
    }

    parts.join("-")
}

fn sanitize_filename_segment(value: &str) -> Option<String> {
    const MAX_CHARS: usize = 48;

    let mut out = String::new();
    let mut last_was_dash = false;
    let mut chars = 0usize;

    for ch in value.chars() {
        if chars >= MAX_CHARS {
            break;
        }

        let is_ok = ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if is_ok {
            out.push(ch);
            last_was_dash = false;
            chars += 1;
            continue;
        }

        if ch.is_whitespace() || ch.is_ascii_punctuation() || ch.is_control() {
            if !out.is_empty() && !last_was_dash {
                out.push('-');
                last_was_dash = true;
                chars += 1;
            }
            continue;
        }

        // Default: treat unsupported chars as separators, but keep the string UTF-8 safe.
        if !out.is_empty() && !last_was_dash {
            out.push('-');
            last_was_dash = true;
            chars += 1;
        }
    }

    let trimmed = out.trim_matches(&['-', '_', '.'][..]).to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn resolve_container(
    path: &Path,
    format: Option<ContainerFormat>,
) -> Result<ContainerFormat, CliError> {
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase());

    let ext_format = match ext.as_deref() {
        Some("mov") => Some(ContainerFormat::Mov),
        Some("mp4") => Some(ContainerFormat::Mp4),
        _ => None,
    };

    if let Some(format) = format {
        if let Some(ext_format) = ext_format {
            if ext_format != format {
                return Err(CliError::usage(format!(
                    "--format {} conflicts with --path extension",
                    format_label(format)
                )));
            }
        }
        return Ok(format);
    }

    Ok(ext_format.unwrap_or(ContainerFormat::Mov))
}

fn format_label(format: ContainerFormat) -> &'static str {
    match format {
        ContainerFormat::Mov => "mov",
        ContainerFormat::Mp4 => "mp4",
    }
}

fn fetch_shareable_content(backend: &Backend) -> Result<ShareableContent, CliError> {
    backend.shareable_content()
}

fn ensure_linux_x11_only_mode_allowed() -> Result<(), CliError> {
    #[cfg(target_os = "linux")]
    {
        match linux_session_kind() {
            LinuxSessionKind::X11 => Ok(()),
            LinuxSessionKind::WaylandOnly => Err(CliError::usage(
                "X11-only mode is unavailable on Wayland-only sessions. Use --portal for recording/screenshot, or log into \"Ubuntu on Xorg\".",
            )),
            LinuxSessionKind::NoDisplay => Err(CliError::runtime(
                "X11 display not detected (DISPLAY is unset).",
            )),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

fn ensure_linux_x11_selectors_allowed() -> Result<(), CliError> {
    #[cfg(target_os = "linux")]
    {
        match linux_session_kind() {
            LinuxSessionKind::X11 => Ok(()),
            LinuxSessionKind::WaylandOnly => Err(CliError::usage(
                "X11 selectors require X11 (DISPLAY is unset). Use --portal on Wayland-only sessions, or log into \"Ubuntu on Xorg\".",
            )),
            LinuxSessionKind::NoDisplay => Err(CliError::runtime(
                "X11 display not detected (DISPLAY is unset).",
            )),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxSessionKind {
    X11,
    WaylandOnly,
    NoDisplay,
}

#[cfg(target_os = "linux")]
fn linux_session_kind() -> LinuxSessionKind {
    let display = std::env::var_os("DISPLAY")
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty());
    if display.is_some() {
        return LinuxSessionKind::X11;
    }

    let wayland = std::env::var_os("WAYLAND_DISPLAY")
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty());
    if wayland.is_some() {
        return LinuxSessionKind::WaylandOnly;
    }

    LinuxSessionKind::NoDisplay
}

fn normalize_app_list(apps: &mut Vec<AppInfo>) {
    apps.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.pid.cmp(&b.pid)));
    apps.dedup_by(|a, b| a.name == b.name && a.pid == b.pid && a.bundle_id == b.bundle_id);
}

fn format_app_tsv(app: &AppInfo) -> String {
    format!(
        "{}\t{}\t{}",
        normalize_tsv_field(&app.name),
        app.pid,
        normalize_tsv_field(&app.bundle_id)
    )
}

fn format_display_tsv(display: &DisplayInfo) -> String {
    format!("{}\t{}\t{}", display.id, display.width, display.height)
}

fn normalize_tsv_field(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\t' || ch == '\n' || ch == '\r' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_record_cli() -> Cli {
        Cli {
            screenshot: false,
            list_windows: false,
            list_displays: false,
            list_apps: false,
            preflight: false,
            request_permission: false,
            window_id: None,
            app: None,
            window_name: None,
            active_window: false,
            display: false,
            display_id: None,
            portal: false,
            duration: Some(1),
            audio: AudioMode::Off,
            path: Some(PathBuf::from("out.mp4")),
            format: None,
            image_format: None,
            dir: None,
        }
    }

    #[test]
    fn portal_requires_audio_off() {
        let mut cli = base_record_cli();
        cli.portal = true;
        cli.audio = AudioMode::System;
        let err = validate_record_args(&cli).expect_err("usage error");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("--portal does not support audio"));
    }
}
