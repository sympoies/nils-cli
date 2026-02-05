use std::path::{Path, PathBuf};

use crate::cli::{AudioMode, Cli, ContainerFormat};
use crate::error::CliError;
use crate::select::{format_window_tsv, select_window, SelectionArgs};
use crate::test_mode;
use crate::types::{AppInfo, ShareableContent};

pub fn run(cli: Cli) -> Result<(), CliError> {
    let test_mode_enabled = test_mode::enabled();
    let mode = determine_mode(&cli)?;

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

    if modes.len() > 1 {
        return Err(CliError::usage(
            "select exactly one mode: --list-windows, --list-apps, --preflight, or --request-permission",
        ));
    }

    Ok(modes.pop().unwrap_or(Mode::Record))
}

fn validate_record_args(cli: &Cli) -> Result<(), CliError> {
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

fn ensure_no_recording_flags(cli: &Cli) -> Result<(), CliError> {
    if cli.window_id.is_some()
        || cli.app.is_some()
        || cli.window_name.is_some()
        || cli.active_window
        || cli.duration.is_some()
        || cli.path.is_some()
        || cli.format.is_some()
        || cli.audio != AudioMode::Off
    {
        return Err(CliError::usage(
            "recording flags are not valid with this mode",
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
