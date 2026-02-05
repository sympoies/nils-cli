use std::path::{Path, PathBuf};

use chrono::Local;

use crate::cli::{AudioMode, Cli, ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::select::{format_window_tsv, select_window, SelectionArgs};
use crate::test_mode;
use crate::types::{AppInfo, ShareableContent, WindowInfo};

pub fn run(cli: Cli) -> Result<(), CliError> {
    let test_mode_enabled = test_mode::enabled();
    let mode = determine_mode(&cli)?;
    validate_screenshot_flag_usage(&cli, mode)?;

    if !test_mode_enabled && !cfg!(target_os = "macos") {
        return Err(CliError::usage("screen-record is only supported on macOS"));
    }

    match mode {
        Mode::Preflight => {
            ensure_no_recording_flags(&cli)?;
            if test_mode_enabled {
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            {
                crate::macos::permissions::preflight()?;
                return Ok(());
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(CliError::usage("screen-record is only supported on macOS"));
            }
        }
        Mode::RequestPermission => {
            ensure_no_recording_flags(&cli)?;
            if test_mode_enabled {
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            {
                crate::macos::permissions::request_permission()?;
                return Ok(());
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(CliError::usage("screen-record is only supported on macOS"));
            }
        }
        Mode::ListWindows => {
            ensure_no_recording_flags(&cli)?;
            let content = fetch_shareable_content(test_mode_enabled)?;
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
        Mode::ListApps => {
            ensure_no_recording_flags(&cli)?;
            let content = fetch_shareable_content(test_mode_enabled)?;
            let mut apps = content.apps;
            normalize_app_list(&mut apps);
            for app in apps {
                println!("{}", format_app_tsv(&app));
            }
        }
        Mode::Screenshot => {
            validate_screenshot_args(&cli)?;

            let content = fetch_shareable_content(test_mode_enabled)?;
            let args = SelectionArgs {
                window_id: cli.window_id,
                app: cli.app.clone(),
                window_name: cli.window_name.clone(),
                active_window: cli.active_window,
            };
            let selected = select_window(&content.windows, &args)?;

            let (output_path, format) =
                resolve_screenshot_output(&cli, &selected, test_mode_enabled)?;

            if test_mode_enabled {
                test_mode::screenshot_fixture(&output_path, format)?;
                println!("{}", output_path.display());
                return Ok(());
            }

            #[cfg(target_os = "macos")]
            {
                crate::macos::screenshot::screenshot_window(&selected, &output_path, format)?;
                println!("{}", output_path.display());
                return Ok(());
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = selected;
                return Err(CliError::usage("screen-record is only supported on macOS"));
            }
        }
        Mode::Record => {
            validate_record_args(&cli)?;

            let content = fetch_shareable_content(test_mode_enabled)?;
            let args = SelectionArgs {
                window_id: cli.window_id,
                app: cli.app.clone(),
                window_name: cli.window_name.clone(),
                active_window: cli.active_window,
            };
            let selected = select_window(&content.windows, &args)?;

            let output_path = resolve_output_path(&cli)?;
            let container = resolve_container(&output_path, cli.format)?;
            if cli.audio == AudioMode::Both && container == ContainerFormat::Mp4 {
                return Err(CliError::usage("--audio both requires .mov"));
            }

            if test_mode_enabled {
                test_mode::record_fixture(&output_path, container)?;
                println!("{}", output_path.display());
                return Ok(());
            }

            #[cfg(target_os = "macos")]
            {
                crate::macos::stream::record_window(
                    &selected,
                    cli.duration.expect("duration validated"),
                    cli.audio,
                    &output_path,
                    container,
                )?;
                println!("{}", output_path.display());
                return Ok(());
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = selected;
                return Err(CliError::usage("screen-record is only supported on macOS"));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ListWindows,
    ListApps,
    Preflight,
    RequestPermission,
    Screenshot,
    Record,
}

fn determine_mode(cli: &Cli) -> Result<Mode, CliError> {
    let mut modes = Vec::new();
    if cli.list_windows {
        modes.push(Mode::ListWindows);
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
            "select exactly one mode: --list-windows, --list-apps, --preflight, --request-permission, or --screenshot",
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

fn validate_record_args(cli: &Cli) -> Result<(), CliError> {
    if cli.image_format.is_some() || cli.dir.is_some() {
        return Err(CliError::usage("screenshot flags require --screenshot"));
    }

    let selection_count =
        cli.window_id.is_some() as u8 + cli.active_window as u8 + cli.app.is_some() as u8;

    if selection_count != 1 {
        return Err(CliError::usage(
            "recording requires exactly one selector: --window-id, --active-window, or --app",
        ));
    }

    if cli.window_name.is_some() && cli.app.is_none() {
        return Err(CliError::usage("--window-name requires --app"));
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

    let selection_count =
        cli.window_id.is_some() as u8 + cli.active_window as u8 + cli.app.is_some() as u8;

    if selection_count != 1 {
        return Err(CliError::usage(
            "screenshot requires exactly one selector: --window-id, --active-window, or --app",
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
    if cli.window_id.is_some()
        || cli.app.is_some()
        || cli.window_name.is_some()
        || cli.active_window
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

fn fetch_shareable_content(test_mode_enabled: bool) -> Result<ShareableContent, CliError> {
    if test_mode_enabled {
        return Ok(test_mode::shareable_content());
    }

    #[cfg(target_os = "macos")]
    {
        crate::macos::shareable::fetch_shareable()
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(CliError::usage("screen-record is only supported on macOS"))
    }
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
