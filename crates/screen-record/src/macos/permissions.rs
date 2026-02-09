use std::process::Command;

use crate::error::CliError;

pub fn preflight() -> Result<(), CliError> {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    Err(permission_error())
}

pub fn request_permission() -> Result<(), CliError> {
    let granted = unsafe { CGRequestScreenCaptureAccess() };
    if !granted {
        open_privacy_pane();
    }
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    Err(permission_error())
}

fn permission_error() -> CliError {
    CliError::runtime(
        "Screen Recording permission not granted. Open System Settings > Privacy & Security > Screen Recording.",
    )
}

fn open_privacy_pane() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenRecording")
        .status();
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}
