use std::env;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use nils_common::process::find_in_path;

use crate::error::CliError;
use crate::linux::portal;

pub fn preflight() -> Result<(), CliError> {
    if find_in_path("ffmpeg").is_none() {
        return Err(CliError::runtime(
            "ffmpeg not found on PATH. Install it with: sudo apt-get install ffmpeg",
        ));
    }

    let display = env::var("DISPLAY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if display.is_none() {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            portal::ensure_portal_available()?;
            return Ok(());
        }
        return Err(CliError::runtime(
            "X11 display not detected (DISPLAY is unset).",
        ));
    }

    if let Some(path) = x11_socket_path(display.as_deref().unwrap_or_default()) {
        if let Err(err) = UnixStream::connect(&path) {
            return Err(CliError::runtime(format!(
                "failed to connect to X11 display (DISPLAY={}): {err}",
                display.as_deref().unwrap_or_default()
            )));
        }
    }

    Ok(())
}

pub fn request_permission() -> Result<(), CliError> {
    preflight()
}

fn x11_socket_path(display: &str) -> Option<PathBuf> {
    let display = display.trim();
    if display.is_empty() {
        return None;
    }

    let mut parts = display.splitn(2, ':');
    let host = parts.next().unwrap_or_default();
    let rest = parts.next()?;

    if !host.is_empty() && host != "unix" && host != "localhost" {
        return None;
    }

    let display_num = rest.split('.').next().unwrap_or_default();
    let display_num = display_num.parse::<u32>().ok()?;
    Some(PathBuf::from(format!("/tmp/.X11-unix/X{display_num}")))
}
